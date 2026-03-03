/// Intent → execution plan compilation.
///
/// This module transforms a high-level [`StrategyExecutionIntent`] — which specifies a strategy
/// template key, action ID, and JSON-typed call arguments — into a concrete [`ExecutionPlan`]
/// containing ABI-encoded calldata ready for on-chain submission.
///
/// # Compilation pipeline (`compile_intent`)
///
/// 1. **Template lookup** — retrieve the [`StrategyTemplate`] from the registry; error if absent.
/// 2. **Action resolution** — find the named action within the template's `actions` list.
/// 3. **Parameter parsing** — deserialise `typed_params_json` into per-call arg arrays; assert
///    call-count parity with `action.call_sequence`.
/// 4. **Role binding** — for each call in the sequence, resolve the contract role to an EVM
///    address via the template's `contract_roles`.
/// 5. **ABI verification** — load the [`AbiArtifact`] for the role; confirm the function
///    signature appears in the artifact (via [`abi::verify_function_selector`]).
/// 6. **ABI encoding** — encode each call's arguments using the full Solidity ABI encoding
///    rules (head/tail layout, dynamic types, tuple recursion).
/// 7. **Plan assembly** — concatenate selector + encoded args into `data`, attach `value_wei`,
///    and collect everything into an [`ExecutionPlan`].
///
/// [`StrategyExecutionIntent`]: crate::domain::types::StrategyExecutionIntent
/// [`ExecutionPlan`]: crate::domain::types::ExecutionPlan
/// [`StrategyTemplate`]: crate::domain::types::StrategyTemplate
/// [`AbiArtifact`]: crate::domain::types::AbiArtifact
use crate::domain::types::{
    AbiArtifactKey, AbiTypeSpec, ExecutionPlan, StrategyExecutionCall, StrategyExecutionIntent,
    StrategyTemplateKey,
};
use crate::strategy::{abi, registry};
use crate::util::{normalize_evm_address, normalize_hex_blob, normalize_selector_hex};
use alloy_primitives::U256;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;

// ── Internal deserialization types ──────────────────────────────────────────

/// Top-level typed parameters extracted from `intent.typed_params_json`.
#[derive(Clone, Debug, Deserialize, Default)]
struct IntentTypedParams {
    #[serde(default)]
    calls: Vec<IntentTypedCall>,
}

/// Per-call arguments supplied by the caller inside `typed_params_json`.
#[derive(Clone, Debug, Deserialize, Default)]
struct IntentTypedCall {
    #[serde(default)]
    args: Vec<Value>,
    #[serde(default)]
    value_wei: Option<String>,
}

// ── Public surface ───────────────────────────────────────────────────────────

/// Compile a [`StrategyExecutionIntent`] into a fully ABI-encoded [`ExecutionPlan`].
///
/// See the module-level documentation for a description of the compilation pipeline.
/// Returns `Err` with a descriptive message if any step fails; the error is safe to
/// surface to callers and is used by the learner to classify failure determinism.
pub fn compile_intent(intent: &StrategyExecutionIntent) -> Result<ExecutionPlan, String> {
    let action_id = normalize_non_empty(&intent.action_id, "action_id")?;
    let template = registry::get_template(&intent.key).ok_or_else(|| {
        format!(
            "strategy template not found for {}:{}:{}:{}",
            intent.key.protocol, intent.key.primitive, intent.key.chain_id, intent.key.template_id
        )
    })?;
    let action = template
        .actions
        .iter()
        .find(|candidate| candidate.action_id == action_id)
        .ok_or_else(|| format!("strategy action not found: {action_id}"))?;
    if action.call_sequence.is_empty() {
        return Err(format!(
            "strategy action {action_id} has an empty call_sequence"
        ));
    }

    // Each element of `typed.calls` must correspond 1:1 with `action.call_sequence`.
    let typed: IntentTypedParams = serde_json::from_str(&intent.typed_params_json)
        .map_err(|error| format!("invalid typed_params_json: {error}"))?;
    if typed.calls.len() != action.call_sequence.len() {
        return Err(format!(
            "call count mismatch for action {action_id}: expected {} got {}",
            action.call_sequence.len(),
            typed.calls.len()
        ));
    }

    // Build a role→binding map for O(1) lookups during call assembly.
    let role_bindings = template
        .contract_roles
        .iter()
        .map(|binding| {
            Ok((
                normalize_non_empty(&binding.role, "contract role")?,
                binding.clone(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let mut calls = Vec::with_capacity(action.call_sequence.len());
    for (index, function) in action.call_sequence.iter().enumerate() {
        let signature = abi::verify_function_selector(function)?;
        let normalized_selector = normalize_selector_hex(&function.selector_hex)?;
        let role = normalize_non_empty(&function.role, "call role")?;
        let binding = role_bindings
            .get(&role)
            .ok_or_else(|| format!("contract role binding not found for role: {role}"))?;
        if binding.source_ref.trim().is_empty() {
            return Err(format!("missing source_ref for role binding: {role}"));
        }
        let to = normalize_evm_address(&binding.address)?;

        let artifact_key = AbiArtifactKey {
            protocol: intent.key.protocol.clone(),
            chain_id: intent.key.chain_id,
            role: role.clone(),
        };
        let artifact = registry::get_abi_artifact(&artifact_key).ok_or_else(|| {
            format!(
                "abi artifact missing for protocol={} role={} chain_id={}",
                artifact_key.protocol, artifact_key.role, artifact_key.chain_id
            )
        })?;
        if artifact.source_ref.trim().is_empty() {
            return Err(format!(
                "abi artifact source_ref missing for role={}",
                artifact_key.role
            ));
        }
        let has_matching_fn = artifact
            .functions
            .iter()
            .any(|candidate| candidate.selector_hex == normalized_selector);
        if !has_matching_fn {
            return Err(format!(
                "abi artifact for role={role} missing function signature {signature}"
            ));
        }

        let typed_call = typed.calls.get(index).ok_or_else(|| {
            format!("typed params call index {index} is missing for action {action_id}")
        })?;
        if typed_call.args.len() != function.inputs.len() {
            return Err(format!(
                "argument count mismatch for call {index} ({signature}): expected {} got {}",
                function.inputs.len(),
                typed_call.args.len()
            ));
        }

        let value_wei = parse_u256_from_decimal_or_hex(
            typed_call.value_wei.as_deref().unwrap_or("0"),
            "value_wei",
        )?
        .to_string();
        let encoded_args = encode_abi_params(&function.inputs, &typed_call.args)?;
        // Calldata = 4-byte selector || ABI-encoded arguments (no length prefix).
        let data = format!(
            "0x{}{}",
            normalized_selector.trim_start_matches("0x"),
            hex::encode(encoded_args)
        );

        calls.push(StrategyExecutionCall {
            role,
            to,
            value_wei,
            data,
        });
    }

    Ok(ExecutionPlan {
        key: intent.key.clone(),
        action_id,
        calls,
        preconditions: action.preconditions.clone(),
        postconditions: action.postconditions.clone(),
    })
}

/// Run a full compile-path validation for a template by compiling a synthetic intent.
///
/// The dry-run uses the template's first action and injects zero-value arguments for every
/// ABI input so `compile_intent` exercises template lookup, role binding, artifact checks,
/// selector verification, and ABI encoding end-to-end.
pub fn dry_run_compile(key: &StrategyTemplateKey) -> Result<(), String> {
    let template = registry::get_template(key).ok_or_else(|| {
        format!(
            "strategy template not found for {}:{}:{}:{}",
            key.protocol, key.primitive, key.chain_id, key.template_id
        )
    })?;
    let first_action = template
        .actions
        .first()
        .ok_or_else(|| "template has no actions".to_string())?;
    if first_action.call_sequence.is_empty() {
        return Err(format!(
            "template action {} has an empty call_sequence",
            first_action.action_id
        ));
    }

    let mut calls = Vec::with_capacity(first_action.call_sequence.len());
    for function in &first_action.call_sequence {
        let mut args = Vec::with_capacity(function.inputs.len());
        for input in &function.inputs {
            args.push(synthetic_zero_value(input)?);
        }
        calls.push(json!({
            "args": args,
            "value_wei": "0",
        }));
    }

    let intent = StrategyExecutionIntent {
        key: key.clone(),
        action_id: first_action.action_id.clone(),
        typed_params_json: json!({ "calls": calls }).to_string(),
    };
    compile_intent(&intent).map(|_| ()).map_err(|error| {
        format!(
            "dry-run compile failed for {}:{}:{}:{} action={}: {error}",
            key.protocol, key.primitive, key.chain_id, key.template_id, first_action.action_id
        )
    })
}

// ── Normalisation helpers ────────────────────────────────────────────────────

fn normalize_non_empty(raw: &str, field: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must be non-empty"));
    }
    Ok(trimmed.to_string())
}

fn parse_u256_from_decimal_or_hex(raw: &str, field: &str) -> Result<U256, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Ok(U256::ZERO);
        }
        if !hex.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{field} must be valid hex"));
        }
        return U256::from_str_radix(hex, 16)
            .map_err(|error| format!("failed to parse {field} as hex quantity: {error}"));
    }
    if !trimmed.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field} must be a decimal string or hex quantity"));
    }
    U256::from_str(trimmed).map_err(|error| format!("failed to parse {field}: {error}"))
}

fn parse_tuple_values<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{field} must be a JSON array"))
}

fn synthetic_zero_value(spec: &AbiTypeSpec) -> Result<Value, String> {
    if let Some((element_kind, maybe_len)) = split_array_type(spec.kind.trim()) {
        let element_spec = AbiTypeSpec {
            kind: element_kind,
            components: spec.components.clone(),
        };
        let len = maybe_len.unwrap_or(0);
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push(synthetic_zero_value(&element_spec)?);
        }
        return Ok(Value::Array(values));
    }

    let kind = spec.kind.trim().to_ascii_lowercase();
    if kind == "tuple" {
        let mut values = Vec::with_capacity(spec.components.len());
        for component in &spec.components {
            values.push(synthetic_zero_value(component)?);
        }
        return Ok(Value::Array(values));
    }

    match kind.as_str() {
        "address" => Ok(Value::String(
            "0x0000000000000000000000000000000000000000".to_string(),
        )),
        "bool" => Ok(Value::Bool(false)),
        "string" => Ok(Value::String(String::new())),
        "bytes" => Ok(Value::String("0x".to_string())),
        _ if kind.starts_with("uint") => Ok(Value::String("0".to_string())),
        _ if kind.starts_with("int") => Ok(Value::String("0".to_string())),
        _ if kind.starts_with("bytes") => Ok(Value::String("0x".to_string())),
        _ => Err(format!(
            "unsupported abi type for synthetic dry-run arg: {}",
            spec.kind
        )),
    }
}

fn split_array_type(kind: &str) -> Option<(String, Option<usize>)> {
    if !kind.ends_with(']') {
        return None;
    }
    let start = kind.rfind('[')?;
    let base = kind[..start].to_string();
    let len_raw = &kind[start + 1..kind.len().saturating_sub(1)];
    if len_raw.is_empty() {
        return Some((base, None));
    }
    len_raw.parse::<usize>().ok().map(|len| (base, Some(len)))
}

fn is_dynamic_type(spec: &AbiTypeSpec) -> Result<bool, String> {
    Ok(static_word_size(spec)?.is_none())
}

fn static_word_size(spec: &AbiTypeSpec) -> Result<Option<usize>, String> {
    if let Some((element_kind, maybe_len)) = split_array_type(spec.kind.trim()) {
        let Some(array_len) = maybe_len else {
            return Ok(None);
        };
        let element = AbiTypeSpec {
            kind: element_kind,
            components: spec.components.clone(),
        };
        let Some(element_words) = static_word_size(&element)? else {
            return Ok(None);
        };
        return Ok(Some(element_words.saturating_mul(array_len)));
    }

    let kind = spec.kind.trim().to_ascii_lowercase();
    if kind == "string" || kind == "bytes" {
        return Ok(None);
    }
    if kind == "tuple" {
        let mut words = 0usize;
        for component in &spec.components {
            let Some(component_words) = static_word_size(component)? else {
                return Ok(None);
            };
            words = words.saturating_add(component_words);
        }
        return Ok(Some(words));
    }
    Ok(Some(1))
}

// ── ABI encoding ─────────────────────────────────────────────────────────────

/// Encode a slice of typed values according to the Solidity ABI head/tail layout.
///
/// Dynamic types (arrays with unknown length, `bytes`, `string`) contribute a 32-byte
/// offset word to the head section and append their payload to the tail.  Static types
/// (fixed-size scalars, fixed-length arrays, static tuples) are written directly into
/// the head.
fn encode_abi_params(specs: &[AbiTypeSpec], values: &[Value]) -> Result<Vec<u8>, String> {
    if specs.len() != values.len() {
        return Err(format!(
            "abi encode arity mismatch: expected {} values, got {}",
            specs.len(),
            values.len()
        ));
    }

    // First pass: compute head section size so tail offsets can be pre-calculated.
    let mut head_size_words = 0usize;
    for spec in specs {
        if is_dynamic_type(spec)? {
            // Dynamic types each reserve exactly one 32-byte offset word in the head.
            head_size_words = head_size_words.saturating_add(1);
        } else {
            let Some(words) = static_word_size(spec)? else {
                return Err("failed to compute static abi word size".to_string());
            };
            head_size_words = head_size_words.saturating_add(words);
        }
    }

    let head_size_bytes = head_size_words.saturating_mul(32);
    let mut heads: Vec<Vec<u8>> = Vec::with_capacity(specs.len());
    let mut tails: Vec<Vec<u8>> = Vec::new();
    let mut tail_size_bytes = 0usize;

    for (index, (spec, value)) in specs.iter().zip(values.iter()).enumerate() {
        if is_dynamic_type(spec)? {
            let tail = encode_abi_dynamic(spec, value, &format!("arg[{index}]"))?;
            let offset = head_size_bytes.saturating_add(tail_size_bytes);
            heads.push(encode_u256_word(U256::from(offset)));
            tail_size_bytes = tail_size_bytes.saturating_add(tail.len());
            tails.push(tail);
        } else {
            heads.push(encode_abi_static(spec, value, &format!("arg[{index}]"))?);
        }
    }

    let mut out = Vec::with_capacity(head_size_bytes.saturating_add(tail_size_bytes));
    for head in heads {
        out.extend_from_slice(&head);
    }
    for tail in tails {
        out.extend_from_slice(&tail);
    }
    Ok(out)
}

fn encode_abi_static(spec: &AbiTypeSpec, value: &Value, field: &str) -> Result<Vec<u8>, String> {
    if is_dynamic_type(spec)? {
        return Err(format!(
            "{field} is dynamic and cannot be encoded as static"
        ));
    }

    if let Some((element_kind, Some(array_len))) = split_array_type(spec.kind.trim()) {
        let values = value
            .as_array()
            .ok_or_else(|| format!("{field} must be an array for fixed-size ABI array"))?;
        if values.len() != array_len {
            return Err(format!(
                "{field} length mismatch: expected {array_len} got {}",
                values.len()
            ));
        }
        let element_spec = AbiTypeSpec {
            kind: element_kind,
            components: spec.components.clone(),
        };
        let mut out = Vec::new();
        for (idx, item) in values.iter().enumerate() {
            out.extend_from_slice(&encode_abi_static(
                &element_spec,
                item,
                &format!("{field}[{idx}]"),
            )?);
        }
        return Ok(out);
    }

    let kind = spec.kind.trim().to_ascii_lowercase();
    if kind == "tuple" {
        let values = parse_tuple_values(value, field)?;
        if values.len() != spec.components.len() {
            return Err(format!(
                "{field} tuple arity mismatch: expected {} got {}",
                spec.components.len(),
                values.len()
            ));
        }
        let mut out = Vec::new();
        for (idx, (component, component_value)) in
            spec.components.iter().zip(values.iter()).enumerate()
        {
            out.extend_from_slice(&encode_abi_static(
                component,
                component_value,
                &format!("{field}.{idx}"),
            )?);
        }
        return Ok(out);
    }

    encode_abi_primitive_word(&kind, value, field)
}

fn encode_abi_dynamic(spec: &AbiTypeSpec, value: &Value, field: &str) -> Result<Vec<u8>, String> {
    if !is_dynamic_type(spec)? {
        return Err(format!(
            "{field} is static and cannot be encoded as dynamic"
        ));
    }

    if let Some((element_kind, maybe_len)) = split_array_type(spec.kind.trim()) {
        let values = value
            .as_array()
            .ok_or_else(|| format!("{field} must be an array for ABI array type"))?;
        if let Some(expected_len) = maybe_len {
            if values.len() != expected_len {
                return Err(format!(
                    "{field} length mismatch: expected {expected_len} got {}",
                    values.len()
                ));
            }
        }
        let element_spec = AbiTypeSpec {
            kind: element_kind,
            components: spec.components.clone(),
        };
        let mut repeated_specs = Vec::with_capacity(values.len());
        for _ in 0..values.len() {
            repeated_specs.push(element_spec.clone());
        }
        let encoded_elements = encode_abi_params(&repeated_specs, values)?;
        let mut out = Vec::new();
        if maybe_len.is_none() {
            out.extend_from_slice(&encode_u256_word(U256::from(values.len())));
        }
        out.extend_from_slice(&encoded_elements);
        return Ok(out);
    }

    let kind = spec.kind.trim().to_ascii_lowercase();
    if kind == "tuple" {
        let values = parse_tuple_values(value, field)?;
        return encode_abi_params(&spec.components, values);
    }
    if kind == "bytes" {
        let raw = value
            .as_str()
            .ok_or_else(|| format!("{field} must be a 0x-prefixed hex string"))?;
        let normalized = normalize_hex_blob(raw, field)?;
        let bytes = hex::decode(normalized.trim_start_matches("0x"))
            .map_err(|error| format!("failed to decode {field}: {error}"))?;
        return Ok(encode_dynamic_bytes(&bytes));
    }
    if kind == "string" {
        let text = value
            .as_str()
            .ok_or_else(|| format!("{field} must be a string"))?;
        return Ok(encode_dynamic_bytes(text.as_bytes()));
    }
    Err(format!("unsupported dynamic abi type: {kind}"))
}

/// Encode a byte slice as an ABI dynamic-bytes value: length word followed by
/// the payload zero-padded to the next 32-byte boundary.
fn encode_dynamic_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&encode_u256_word(U256::from(bytes.len())));
    out.extend_from_slice(bytes);
    // Pad to 32-byte boundary; `(32 - len % 32) % 32` handles exact multiples correctly.
    let padding = (32usize.saturating_sub(bytes.len() % 32)) % 32;
    if padding > 0 {
        out.extend(vec![0u8; padding]);
    }
    out
}

fn encode_abi_primitive_word(kind: &str, value: &Value, field: &str) -> Result<Vec<u8>, String> {
    match kind {
        "address" => {
            let raw = value
                .as_str()
                .ok_or_else(|| format!("{field} address must be a string"))?;
            let normalized = normalize_evm_address(raw)?;
            let mut word = vec![0u8; 32];
            let bytes = hex::decode(normalized.trim_start_matches("0x"))
                .map_err(|error| format!("failed to decode {field} address: {error}"))?;
            word[12..].copy_from_slice(&bytes);
            Ok(word)
        }
        "bool" => {
            let raw = value
                .as_bool()
                .ok_or_else(|| format!("{field} bool must be true/false"))?;
            Ok(encode_u256_word(U256::from(u8::from(raw))))
        }
        _ if kind.starts_with("uint") => {
            let parsed = parse_u256_from_json(value, field)?;
            Ok(encode_u256_word(parsed))
        }
        _ if kind.starts_with("int") => {
            let parsed = parse_i128_from_json(value, field)?;
            if parsed < 0 {
                return Err(format!(
                    "{field} negative signed integers are not supported yet"
                ));
            }
            Ok(encode_u256_word(U256::from(parsed as u128)))
        }
        _ if kind.starts_with("bytes") => {
            let width_raw = kind.trim_start_matches("bytes");
            let width = width_raw
                .parse::<usize>()
                .map_err(|_error| format!("unsupported abi type: {kind}"))?;
            if !(1..=32).contains(&width) {
                return Err(format!("fixed bytes width must be in 1..=32, got {width}"));
            }
            let raw = value
                .as_str()
                .ok_or_else(|| format!("{field} fixed bytes must be a hex string"))?;
            let normalized = normalize_hex_blob(raw, field)?;
            let bytes = hex::decode(normalized.trim_start_matches("0x"))
                .map_err(|error| format!("failed to decode {field}: {error}"))?;
            if bytes.len() > width {
                return Err(format!(
                    "{field} length exceeds bytes{width}: {} bytes",
                    bytes.len()
                ));
            }
            let mut word = vec![0u8; 32];
            word[..bytes.len()].copy_from_slice(&bytes);
            Ok(word)
        }
        _ => Err(format!("unsupported abi primitive type: {kind}")),
    }
}

fn parse_u256_from_json(value: &Value, field: &str) -> Result<U256, String> {
    if let Some(raw) = value.as_str() {
        return parse_u256_from_decimal_or_hex(raw, field);
    }
    if let Some(raw) = value.as_u64() {
        return Ok(U256::from(raw));
    }
    Err(format!("{field} must be a string or unsigned integer"))
}

fn parse_i128_from_json(value: &Value, field: &str) -> Result<i128, String> {
    if let Some(raw) = value.as_i64() {
        return Ok(i128::from(raw));
    }
    let raw = value
        .as_str()
        .ok_or_else(|| format!("{field} must be a string or integer"))?;
    raw.parse::<i128>()
        .map_err(|error| format!("failed to parse {field} as signed integer: {error}"))
}

/// Encode a `U256` as a big-endian 32-byte ABI word.
fn encode_u256_word(value: U256) -> Vec<u8> {
    value.to_be_bytes::<32>().to_vec()
}

#[cfg(test)]
mod tests {
    use super::{compile_intent, dry_run_compile};
    use crate::domain::types::{
        AbiArtifact, AbiArtifactKey, AbiFunctionSpec, AbiTypeSpec, ActionSpec, ContractRoleBinding,
        StrategyExecutionIntent, StrategyTemplate, StrategyTemplateKey, TemplateStatus,
    };
    use crate::storage::stable;
    use crate::strategy::registry;
    use alloy_primitives::U256;

    fn sample_key(template_id: &str) -> StrategyTemplateKey {
        StrategyTemplateKey {
            protocol: "erc20".to_string(),
            primitive: "transfer".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        }
    }

    fn transfer_function(role: &str) -> AbiFunctionSpec {
        AbiFunctionSpec {
            role: role.to_string(),
            name: "transfer".to_string(),
            selector_hex: "0xa9059cbb".to_string(),
            inputs: vec![
                AbiTypeSpec {
                    kind: "address".to_string(),
                    components: Vec::new(),
                },
                AbiTypeSpec {
                    kind: "uint256".to_string(),
                    components: Vec::new(),
                },
            ],
            outputs: vec![AbiTypeSpec {
                kind: "bool".to_string(),
                components: Vec::new(),
            }],
            state_mutability: "nonpayable".to_string(),
        }
    }

    fn transfer_function_with_selector(role: &str, selector_hex: &str) -> AbiFunctionSpec {
        let mut function = transfer_function(role);
        function.selector_hex = selector_hex.to_string();
        function
    }

    fn store_template_and_abi_with_selector(template_id: &str, selector_hex: &str) {
        let key = sample_key(template_id);
        let function = transfer_function_with_selector("token", selector_hex);
        let action = ActionSpec {
            action_id: "transfer".to_string(),
            call_sequence: vec![function.clone()],
            preconditions: vec!["allowance_ok".to_string()],
            postconditions: vec!["balance_delta_gt_zero".to_string()],
            risk_checks: vec!["max_notional".to_string()],
        };
        registry::upsert_template(StrategyTemplate {
            key: key.clone(),
            status: TemplateStatus::Active,
            contract_roles: vec![ContractRoleBinding {
                role: "token".to_string(),
                address: "0x2222222222222222222222222222222222222222".to_string(),
                source_ref: "https://example.com/token".to_string(),
                codehash: None,
            }],
            actions: vec![action],
            constraints_json: "{}".to_string(),
            created_at_ns: 1,
            updated_at_ns: 1,
        })
        .expect("template should persist");

        registry::upsert_abi_artifact(AbiArtifact {
            key: AbiArtifactKey {
                protocol: key.protocol.clone(),
                chain_id: key.chain_id,
                role: "token".to_string(),
            },
            source_ref: "https://example.com/token-abi".to_string(),
            codehash: None,
            abi_json: "[]".to_string(),
            functions: vec![function],
            created_at_ns: 1,
            updated_at_ns: 1,
        })
        .expect("abi artifact should persist");
    }

    fn store_template_and_abi(template_id: &str) {
        store_template_and_abi_with_selector(template_id, "0xa9059cbb");
    }

    #[test]
    fn compile_intent_builds_execution_plan_with_deterministic_calldata() {
        stable::init_storage();
        let template_id = "compiler-success";
        store_template_and_abi(template_id);
        let key = sample_key(template_id);

        let intent = StrategyExecutionIntent {
            key: key.clone(),
            action_id: "transfer".to_string(),
            typed_params_json: r#"{"calls":[{"args":["0x3333333333333333333333333333333333333333","1000"],"value_wei":"0"}]}"#
                .to_string(),
        };
        let plan = compile_intent(&intent).expect("intent should compile");
        assert_eq!(plan.key, key);
        assert_eq!(plan.calls.len(), 1);
        assert_eq!(
            plan.calls[0].to,
            "0x2222222222222222222222222222222222222222"
        );
        assert_eq!(plan.calls[0].value_wei, "0");

        let expected_amount = format!("{:064x}", U256::from(1_000u64));
        assert_eq!(
            plan.calls[0].data,
            format!(
                "0xa9059cbb{:0>64}{}",
                "3333333333333333333333333333333333333333", expected_amount
            )
        );
        assert_eq!(plan.preconditions, vec!["allowance_ok"]);
        assert_eq!(plan.postconditions, vec!["balance_delta_gt_zero"]);
    }

    #[test]
    fn compile_intent_rejects_argument_shape_mismatch() {
        stable::init_storage();
        let template_id = "compiler-arg-mismatch";
        store_template_and_abi(template_id);

        let intent = StrategyExecutionIntent {
            key: sample_key(template_id),
            action_id: "transfer".to_string(),
            typed_params_json: r#"{"calls":[{"args":["0x3333333333333333333333333333333333333333"],"value_wei":"0"}]}"#
                .to_string(),
        };
        let err = compile_intent(&intent).expect_err("argument mismatch must fail");
        assert!(
            err.contains("argument count mismatch"),
            "expected argument mismatch error, got {err}"
        );
    }

    #[test]
    fn dry_run_compile_succeeds_with_synthetic_zero_args() {
        stable::init_storage();
        let template_id = "compiler-dry-run-success";
        store_template_and_abi(template_id);

        let result = dry_run_compile(&sample_key(template_id));
        assert!(result.is_ok(), "dry-run compile should pass: {result:?}");
    }

    #[test]
    fn dry_run_compile_fails_when_selector_is_invalid() {
        stable::init_storage();
        let template_id = "compiler-dry-run-selector-fail";
        store_template_and_abi_with_selector(template_id, "0xdeadbeef");

        let err = dry_run_compile(&sample_key(template_id))
            .expect_err("dry-run compile should fail on selector mismatch");
        assert!(
            err.contains("selector mismatch"),
            "expected selector mismatch error, got {err}"
        );
    }
}
