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
    AbiArtifactKey, AbiFunctionSpec, AbiTypeSpec, ActionSpec, ExecutionPlan,
    StrategyAssetDirection, StrategyAssetEffect, StrategyExecutionCall, StrategyExecutionIntent,
    StrategyTemplateKey,
};
use crate::strategy::{abi, registry};
use crate::util::{normalize_evm_address, normalize_hex_blob, normalize_selector_hex};
use alloy_primitives::U256;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
    #[serde(default = "default_call_args")]
    args: Value,
    #[serde(default)]
    value_wei: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StrategyActionArgumentSchema {
    pub action_id: String,
    pub calls: Vec<StrategyActionCallSchema>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StrategyActionCallSchema {
    pub role: String,
    pub function_name: String,
    pub signature: String,
    pub value_allowed: bool,
    pub args: Vec<AbiTypeSpec>,
}

// ── Public surface ───────────────────────────────────────────────────────────

/// Compile a [`StrategyExecutionIntent`] into a fully ABI-encoded [`ExecutionPlan`].
///
/// See the module-level documentation for a description of the compilation pipeline.
/// Returns `Err` with a descriptive message if any step fails; the error is safe to
/// surface to callers and is used by the learner to classify failure determinism.
pub fn compile_intent(intent: &StrategyExecutionIntent) -> Result<ExecutionPlan, String> {
    let action_id = normalize_non_empty(&intent.action_id, "action_id")?;
    let template = load_template(&intent.key)?;
    let action = resolve_action(&template.actions, &action_id)?;
    let action_schema = derive_action_argument_schema_from_action(action)?;
    if action.call_sequence.is_empty() {
        return Err(format!(
            "strategy action {action_id} has an empty call_sequence"
        ));
    }

    // Each element of `typed.calls` must correspond 1:1 with `action.call_sequence`.
    let typed: IntentTypedParams = serde_json::from_str(&intent.typed_params_json)
        .map_err(|error| format!("invalid typed_params_json: {error}"))?;
    let typed_value: Value = serde_json::from_str(&intent.typed_params_json)
        .map_err(|error| format!("invalid typed_params_json: {error}"))?;
    if typed.calls.len() != action_schema.calls.len() {
        return Err(format!(
            "call count mismatch for action {action_id}: expected {} got {}",
            action_schema.calls.len(),
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
    for (index, (function, call_schema)) in action
        .call_sequence
        .iter()
        .zip(action_schema.calls.iter())
        .enumerate()
    {
        let signature = &call_schema.signature;
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
        let value_wei = parse_u256_from_decimal_or_hex(
            typed_call.value_wei.as_deref().unwrap_or("0"),
            "value_wei",
        )?
        .to_string();
        let lowered_args = lower_call_args(
            &typed_call.args,
            &call_schema.args,
            &format!("calls[{index}].args"),
        )?;
        let encoded_args = encode_abi_params(&call_schema.args, &lowered_args)?;
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

    let constraints: Value = serde_json::from_str(&template.constraints_json)
        .map_err(|error| format!("invalid template constraints_json: {error}"))?;
    let declarations: Vec<crate::domain::types::StrategyAssetEffectDeclaration> = constraints
        .get("asset_effects")
        .and_then(|value| value.get(&action_id))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("strategy action {action_id} has invalid asset_effects: {error}"))?
        .unwrap_or_default();
    let mut asset_effects = Vec::new();
    for declaration in declarations {
        if declaration.decimals > 36 {
            return Err(format!(
                "strategy action {action_id} has unsupported asset_effects.decimals {}",
                declaration.decimals
            ));
        }
        let amount = resolve_effect_amount(&typed_value, &declaration.amount_path, &action_schema)
            .ok_or_else(|| {
                format!(
                    "strategy action {action_id} asset effect amount path `{}` is missing",
                    declaration.amount_path
                )
            })?;
        let amount_raw = match amount {
            Value::String(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => value.clone(),
            Value::Number(value) if value.is_u64() => value.to_string(),
            _ => return Err(format!("strategy action {action_id} asset effect amount path `{}` must be an unsigned integer", declaration.amount_path)),
        };
        let asset_address = declaration.asset_role.as_deref().map(|role| {
            role_bindings.get(role).ok_or_else(|| format!("strategy action {action_id} asset effect references unknown role `{role}`"))
                .and_then(|binding| normalize_evm_address(&binding.address))
        }).transpose()?;
        let effect = StrategyAssetEffect {
            chain_id: intent.key.chain_id,
            asset_address,
            asset_symbol: declaration.asset_symbol,
            decimals: declaration.decimals,
            amount_raw,
            direction: declaration.direction,
        };
        if asset_effects.iter().any(|prior: &StrategyAssetEffect| {
            prior.chain_id == effect.chain_id
                && prior.asset_address == effect.asset_address
                && prior.amount_raw == effect.amount_raw
                && prior.direction != effect.direction
        }) {
            return Err(format!(
                "strategy action {action_id} has contradictory asset effects"
            ));
        }
        asset_effects.push(effect);
    }
    for call in &calls {
        if call.value_wei != "0" {
            asset_effects.push(StrategyAssetEffect {
                chain_id: intent.key.chain_id,
                asset_address: None,
                asset_symbol: "ETH".into(),
                decimals: 18,
                amount_raw: call.value_wei.clone(),
                direction: StrategyAssetDirection::Debit,
            });
        }
    }
    if (action_id.starts_with("enter_") || action_id.starts_with("exit_"))
        && asset_effects.is_empty()
    {
        return Err(format!(
            "strategy action {action_id} is capital-moving but has no asset effect declaration"
        ));
    }
    validate_asset_effect_bindings(&template, &action_id, &calls, &asset_effects)?;
    Ok(ExecutionPlan {
        key: intent.key.clone(),
        action_id,
        calls,
        preconditions: action.preconditions.clone(),
        postconditions: action.postconditions.clone(),
        asset_effects,
        risk_checks: action.risk_checks.clone(),
    })
}

fn validate_asset_effect_bindings(
    template: &crate::domain::types::StrategyTemplate,
    action_id: &str,
    calls: &[StrategyExecutionCall],
    effects: &[StrategyAssetEffect],
) -> Result<(), String> {
    let Some(call) = calls.first() else {
        return Ok(());
    };
    for effect in effects
        .iter()
        .filter(|effect| effect.asset_address.is_some())
    {
        let expected = effect.asset_address.as_deref().unwrap_or_default();
        let bound = match template.key.protocol.as_str() {
            "aave-v3" => calldata_address_word(call, 0)
                .is_some_and(|value| value.eq_ignore_ascii_case(expected)),
            "morpho-v1" => calldata_address_word(call, 0)
                .is_some_and(|value| value.eq_ignore_ascii_case(expected)),
            "moonwell-v2" => {
                call.role == "m_usdc"
                    && template.contract_roles.iter().any(|binding| {
                        binding.role == "usdc" && binding.address.eq_ignore_ascii_case(expected)
                    })
            }
            _ => false,
        };
        if !bound {
            return Err(format!("strategy action {action_id} asset effect is not bound to calldata/target asset {expected}"));
        }
    }
    Ok(())
}

fn calldata_address_word(call: &StrategyExecutionCall, index: usize) -> Option<String> {
    let data = call.data.strip_prefix("0x")?;
    let start = 8usize.checked_add(index.checked_mul(64)?)?;
    let word = data.get(start..start.checked_add(64)?)?;
    Some(format!("0x{}", word.get(24..)?.to_ascii_lowercase()))
}

fn resolve_effect_amount<'a>(
    value: &'a Value,
    path: &str,
    schema: &StrategyActionArgumentSchema,
) -> Option<&'a Value> {
    if let Some(found) = value.pointer(path) {
        return Some(found);
    }
    let segments = path.strip_prefix("/calls/")?.split('/').collect::<Vec<_>>();
    if segments.len() != 3 || segments[1] != "args" {
        return None;
    }
    let call_index = segments[0].parse::<usize>().ok()?;
    let arg_index = segments[2].parse::<usize>().ok()?;
    let name = &schema.calls.get(call_index)?.args.get(arg_index)?.name;
    value.get("calls")?.get(call_index)?.get("args")?.get(name)
}

pub(crate) fn derive_action_argument_schema(
    key: &StrategyTemplateKey,
    action_id: &str,
) -> Result<StrategyActionArgumentSchema, String> {
    let normalized_action_id = normalize_non_empty(action_id, "action_id")?;
    let template = load_template(key)?;
    let action = resolve_action(&template.actions, &normalized_action_id)?;
    derive_action_argument_schema_from_action(action)
}

/// Run a full compile-path validation for a template by compiling a synthetic intent.
///
/// The dry-run uses the template's first action and injects zero-value arguments for every
/// ABI input so `compile_intent` exercises template lookup, role binding, artifact checks,
/// selector verification, and ABI encoding end-to-end.
pub fn dry_run_compile(key: &StrategyTemplateKey) -> Result<(), String> {
    let template = load_template(key)?;
    let first_action = template
        .actions
        .first()
        .ok_or_else(|| "template has no actions".to_string())?;
    let action_schema = derive_action_argument_schema_from_action(first_action)?;
    if first_action.call_sequence.is_empty() {
        return Err(format!(
            "template action {} has an empty call_sequence",
            first_action.action_id
        ));
    }

    let mut calls = Vec::with_capacity(action_schema.calls.len());
    for call_schema in &action_schema.calls {
        let mut args = serde_json::Map::with_capacity(call_schema.args.len());
        for input in &call_schema.args {
            args.insert(input.name.clone(), synthetic_zero_value(input)?);
        }
        calls.push(json!({
            "args": Value::Object(args),
            "value_wei": "0",
        }));
    }
    if let Some(usdc) = template
        .contract_roles
        .iter()
        .find(|binding| binding.role == "usdc")
    {
        match template.key.protocol.as_str() {
            "aave-v3" => {
                if let Some(args) = calls
                    .get_mut(0)
                    .and_then(|call| call.get_mut("args"))
                    .and_then(Value::as_object_mut)
                {
                    args.insert("asset".to_string(), Value::String(usdc.address.clone()));
                }
            }
            "morpho-v1" => {
                if let Some(market) = calls
                    .get_mut(0)
                    .and_then(|call| call.get_mut("args"))
                    .and_then(|args| args.get_mut("marketParams"))
                    .and_then(Value::as_object_mut)
                {
                    market.insert("loanToken".to_string(), Value::String(usdc.address.clone()));
                }
            }
            _ => {}
        }
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

fn default_call_args() -> Value {
    Value::Array(Vec::new())
}

fn load_template(
    key: &StrategyTemplateKey,
) -> Result<crate::domain::types::StrategyTemplate, String> {
    registry::get_template(key).ok_or_else(|| {
        format!(
            "strategy template not found for {}:{}:{}:{}",
            key.protocol, key.primitive, key.chain_id, key.template_id
        )
    })
}

fn resolve_action<'a>(
    actions: &'a [ActionSpec],
    action_id: &str,
) -> Result<&'a ActionSpec, String> {
    actions
        .iter()
        .find(|candidate| candidate.action_id == action_id)
        .ok_or_else(|| format!("strategy action not found: {action_id}"))
}

fn derive_action_argument_schema_from_action(
    action: &ActionSpec,
) -> Result<StrategyActionArgumentSchema, String> {
    derive_action_argument_schema_from_functions(&action.action_id, &action.call_sequence)
}

fn derive_action_argument_schema_from_functions(
    action_id: &str,
    functions: &[AbiFunctionSpec],
) -> Result<StrategyActionArgumentSchema, String> {
    let mut calls = Vec::with_capacity(functions.len());
    for function in functions {
        calls.push(StrategyActionCallSchema {
            role: normalize_non_empty(&function.role, "call role")?,
            function_name: normalize_non_empty(&function.name, "function name")?,
            signature: abi::verify_function_selector(function)?,
            value_allowed: function.state_mutability.eq_ignore_ascii_case("payable"),
            args: normalize_named_specs(&function.inputs),
        });
    }
    Ok(StrategyActionArgumentSchema {
        action_id: normalize_non_empty(action_id, "action_id")?,
        calls,
    })
}

fn normalize_named_specs(specs: &[AbiTypeSpec]) -> Vec<AbiTypeSpec> {
    let mut used_names = BTreeSet::new();
    let mut next_fallback_index = 0usize;
    let mut normalized = Vec::with_capacity(specs.len());
    for spec in specs {
        let name =
            normalize_schema_param_name(&spec.name, &mut used_names, &mut next_fallback_index);
        normalized.push(AbiTypeSpec {
            name,
            kind: spec.kind.clone(),
            components: normalize_named_specs(&spec.components),
        });
    }
    normalized
}

fn normalize_schema_param_name(
    raw_name: &str,
    used_names: &mut BTreeSet<String>,
    next_fallback_index: &mut usize,
) -> String {
    let candidate = raw_name.trim();
    if !candidate.is_empty() && used_names.insert(candidate.to_string()) {
        return candidate.to_string();
    }

    loop {
        let fallback = format!("arg{}", *next_fallback_index);
        *next_fallback_index = next_fallback_index.saturating_add(1);
        if used_names.insert(fallback.clone()) {
            return fallback;
        }
    }
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

fn lower_call_args(args: &Value, specs: &[AbiTypeSpec], field: &str) -> Result<Vec<Value>, String> {
    match args {
        Value::Array(values) => {
            if values.len() != specs.len() {
                return Err(format!(
                    "argument count mismatch for {field}: expected {} got {}",
                    specs.len(),
                    values.len()
                ));
            }
            values
                .iter()
                .zip(specs.iter())
                .map(|(value, spec)| {
                    lower_value_to_canonical_shape(spec, value, &format!("{field}.{}", spec.name))
                })
                .collect()
        }
        Value::Object(object) => {
            reject_unknown_object_fields(object, specs, field)?;
            let mut lowered = Vec::with_capacity(specs.len());
            for spec in specs {
                let child_field = format!("{field}.{}", spec.name);
                let value = object
                    .get(&spec.name)
                    .ok_or_else(|| format!("missing required field: {child_field}"))?;
                lowered.push(lower_value_to_canonical_shape(spec, value, &child_field)?);
            }
            Ok(lowered)
        }
        _ => Err(format!("{field} must be a JSON object or array")),
    }
}

fn lower_value_to_canonical_shape(
    spec: &AbiTypeSpec,
    value: &Value,
    field: &str,
) -> Result<Value, String> {
    if let Some((element_kind, maybe_len)) = split_array_type(spec.kind.trim()) {
        let values = value
            .as_array()
            .ok_or_else(|| format!("{field} must be a JSON array"))?;
        if let Some(expected_len) = maybe_len {
            if values.len() != expected_len {
                return Err(format!(
                    "{field} length mismatch: expected {expected_len} got {}",
                    values.len()
                ));
            }
        }
        let element_spec = AbiTypeSpec {
            name: spec.name.clone(),
            kind: element_kind,
            components: spec.components.clone(),
        };
        let mut lowered = Vec::with_capacity(values.len());
        for (index, element) in values.iter().enumerate() {
            lowered.push(lower_value_to_canonical_shape(
                &element_spec,
                element,
                &format!("{field}[{index}]"),
            )?);
        }
        return Ok(Value::Array(lowered));
    }

    let kind = spec.kind.trim().to_ascii_lowercase();
    if kind == "tuple" {
        return lower_tuple_value(spec, value, field);
    }

    Ok(value.clone())
}

fn lower_tuple_value(spec: &AbiTypeSpec, value: &Value, field: &str) -> Result<Value, String> {
    match value {
        Value::Array(values) => {
            if values.len() != spec.components.len() {
                return Err(format!(
                    "{field} tuple arity mismatch: expected {} got {}",
                    spec.components.len(),
                    values.len()
                ));
            }
            let mut lowered = Vec::with_capacity(spec.components.len());
            for (component, component_value) in spec.components.iter().zip(values.iter()) {
                lowered.push(lower_value_to_canonical_shape(
                    component,
                    component_value,
                    &format!("{field}.{}", component.name),
                )?);
            }
            Ok(Value::Array(lowered))
        }
        Value::Object(object) => {
            reject_unknown_object_fields(object, &spec.components, field)?;
            let mut lowered = Vec::with_capacity(spec.components.len());
            for component in &spec.components {
                let child_field = format!("{field}.{}", component.name);
                let component_value = object
                    .get(&component.name)
                    .ok_or_else(|| format!("missing required field: {child_field}"))?;
                lowered.push(lower_value_to_canonical_shape(
                    component,
                    component_value,
                    &child_field,
                )?);
            }
            Ok(Value::Array(lowered))
        }
        _ => Err(format!("{field} must be a JSON object or array")),
    }
}

fn reject_unknown_object_fields(
    object: &serde_json::Map<String, Value>,
    specs: &[AbiTypeSpec],
    field: &str,
) -> Result<(), String> {
    let expected_fields = specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut unknown_fields = object
        .keys()
        .filter(|key| !expected_fields.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unknown_fields.sort();
    if let Some(first_unknown) = unknown_fields.first() {
        return Err(format!("unknown field: {field}.{first_unknown}"));
    }
    Ok(())
}

fn synthetic_zero_value(spec: &AbiTypeSpec) -> Result<Value, String> {
    if let Some((element_kind, maybe_len)) = split_array_type(spec.kind.trim()) {
        let element_spec = AbiTypeSpec {
            name: spec.name.clone(),
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
        let mut values = serde_json::Map::with_capacity(spec.components.len());
        for component in &spec.components {
            values.insert(component.name.clone(), synthetic_zero_value(component)?);
        }
        return Ok(Value::Object(values));
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
            name: spec.name.clone(),
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
            name: spec.name.clone(),
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
            name: spec.name.clone(),
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
        StrategyAssetDirection, StrategyExecutionIntent, StrategyTemplate, StrategyTemplateKey,
        TemplateStatus,
    };
    use crate::storage::stable;
    use crate::strategy::registry;
    use alloy_primitives::U256;
    use serde_json::Value;

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
                    name: "to".to_string(),
                    kind: "address".to_string(),
                    components: Vec::new(),
                },
                AbiTypeSpec {
                    name: "amount".to_string(),
                    kind: "uint256".to_string(),
                    components: Vec::new(),
                },
            ],
            outputs: vec![AbiTypeSpec {
                name: "success".to_string(),
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

    // ── Morpho supply regression ─────────────────────────────────────────

    /// Morpho `supply` function spec matching the recipe ABI.
    ///
    /// supply((address,address,address,address,uint256),uint256,uint256,address,bytes)
    /// selector: 0xa99aad89
    fn morpho_supply_function(role: &str) -> AbiFunctionSpec {
        AbiFunctionSpec {
            role: role.to_string(),
            name: "supply".to_string(),
            selector_hex: "0xa99aad89".to_string(),
            inputs: vec![
                AbiTypeSpec {
                    name: "marketParams".to_string(),
                    kind: "tuple".to_string(),
                    components: vec![
                        AbiTypeSpec {
                            name: "loanToken".to_string(),
                            kind: "address".to_string(),
                            components: vec![],
                        },
                        AbiTypeSpec {
                            name: "collateralToken".to_string(),
                            kind: "address".to_string(),
                            components: vec![],
                        },
                        AbiTypeSpec {
                            name: "oracle".to_string(),
                            kind: "address".to_string(),
                            components: vec![],
                        },
                        AbiTypeSpec {
                            name: "irm".to_string(),
                            kind: "address".to_string(),
                            components: vec![],
                        },
                        AbiTypeSpec {
                            name: "lltv".to_string(),
                            kind: "uint256".to_string(),
                            components: vec![],
                        },
                    ],
                },
                AbiTypeSpec {
                    name: "assets".to_string(),
                    kind: "uint256".to_string(),
                    components: vec![],
                },
                AbiTypeSpec {
                    name: "shares".to_string(),
                    kind: "uint256".to_string(),
                    components: vec![],
                },
                AbiTypeSpec {
                    name: "onBehalf".to_string(),
                    kind: "address".to_string(),
                    components: vec![],
                },
                AbiTypeSpec {
                    name: "data".to_string(),
                    kind: "bytes".to_string(),
                    components: vec![],
                },
            ],
            outputs: vec![
                AbiTypeSpec {
                    name: "assetsSupplied".to_string(),
                    kind: "uint256".to_string(),
                    components: vec![],
                },
                AbiTypeSpec {
                    name: "sharesMinted".to_string(),
                    kind: "uint256".to_string(),
                    components: vec![],
                },
            ],
            state_mutability: "nonpayable".to_string(),
        }
    }

    fn store_morpho_template_and_abi(template_id: &str) {
        let key = StrategyTemplateKey {
            protocol: "morpho-v1".to_string(),
            primitive: "lend_supply".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        };
        let function = morpho_supply_function("morpho");
        let action = ActionSpec {
            action_id: "enter_supply".to_string(),
            call_sequence: vec![function.clone()],
            preconditions: vec!["market_supply_apy_gte_0.03".to_string()],
            postconditions: vec!["morpho_supply_position_increased".to_string()],
            risk_checks: vec!["lltv_equals_0.86e18".to_string()],
        };
        registry::upsert_template(StrategyTemplate {
            key: key.clone(),
            status: TemplateStatus::Active,
            contract_roles: vec![ContractRoleBinding {
                role: "morpho".to_string(),
                address: "0xBBBBBbbBBb9cC5e90e3b3Af64bdAF62C37EEFFCb".to_string(),
                source_ref: "https://docs.morpho.org/get-started/resources/addresses/".to_string(),
                codehash: None,
            }, ContractRoleBinding { role: "usdc".to_string(), address: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string(), source_ref: "test".to_string(), codehash: None }],
            actions: vec![action],
            constraints_json: serde_json::json!({"asset_effects":{"enter_supply":[{"asset_role":"usdc","asset_symbol":"USDC","decimals":6,"direction":"Debit","amount_path":"/calls/0/args/1"}]}}).to_string(),
            created_at_ns: 1,
            updated_at_ns: 1,
        })
        .expect("morpho template should persist");

        registry::upsert_abi_artifact(AbiArtifact {
            key: AbiArtifactKey {
                protocol: "morpho-v1".to_string(),
                chain_id: 8453,
                role: "morpho".to_string(),
            },
            source_ref: "https://docs.morpho.org/get-started/resources/addresses/".to_string(),
            codehash: None,
            abi_json: "[]".to_string(),
            functions: vec![function],
            created_at_ns: 1,
            updated_at_ns: 1,
        })
        .expect("morpho abi artifact should persist");
    }

    fn morpho_supply_named_args_json() -> serde_json::Value {
        serde_json::json!({
            "marketParams": {
                "loanToken": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                "collateralToken": "0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf",
                "oracle": "0x663E04CBb82e44A8544828C7C3e2f02820085f00",
                "irm": "0x46415998764C29aB2a25CbeA6E5D2F226b40b5f0",
                "lltv": "860000000000000000"
            },
            "assets": "1000000",
            "shares": "0",
            "onBehalf": "0x1111111111111111111111111111111111111111",
            "data": "0x"
        })
    }

    #[test]
    fn compile_morpho_enter_supply_produces_at_least_one_call() {
        stable::init_storage();
        let template_id = "morpho-supply-regression";
        store_morpho_template_and_abi(template_id);
        let key = StrategyTemplateKey {
            protocol: "morpho-v1".to_string(),
            primitive: "lend_supply".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        };

        let market_params = serde_json::json!([
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf",
            "0x663E04CBb82e44A8544828C7C3e2f02820085f00",
            "0x46415998764C29aB2a25CbeA6E5D2F226b40b5f0",
            "860000000000000000"
        ]);
        let on_behalf = "0x1111111111111111111111111111111111111111";
        let intent = StrategyExecutionIntent {
            key: key.clone(),
            action_id: "enter_supply".to_string(),
            typed_params_json: serde_json::json!({
                "calls": [{
                    "args": [
                        market_params,
                        "1000000",
                        "0",
                        on_behalf,
                        "0x"
                    ],
                    "value_wei": "0"
                }]
            })
            .to_string(),
        };

        let plan = compile_intent(&intent).expect("morpho enter_supply should compile");
        assert_eq!(plan.asset_effects.len(), 1);
        assert_eq!(plan.asset_effects[0].asset_symbol, "USDC");
        assert_eq!(plan.asset_effects[0].amount_raw, "1000000");
        assert_eq!(
            plan.asset_effects[0].direction,
            StrategyAssetDirection::Debit
        );
        assert!(
            !plan.calls.is_empty(),
            "enter_supply must produce at least one compiled call"
        );
        assert_eq!(plan.calls.len(), 1);
        assert_eq!(
            plan.calls[0].to,
            "0xbbbbbbbbbb9cc5e90e3b3af64bdaf62c37eeffcb"
        );
        assert!(
            plan.calls[0].data.starts_with("0xa99aad89"),
            "calldata must start with supply selector, got {}",
            &plan.calls[0].data[..std::cmp::min(10, plan.calls[0].data.len())]
        );
        let mut wrong: Value = serde_json::from_str(&intent.typed_params_json).unwrap();
        *wrong.pointer_mut("/calls/0/args/0/0").unwrap() =
            Value::String("0x2222222222222222222222222222222222222222".to_string());
        let wrong_intent = StrategyExecutionIntent {
            typed_params_json: wrong.to_string(),
            ..intent
        };
        assert!(compile_intent(&wrong_intent)
            .unwrap_err()
            .contains("not bound to calldata/target"));
    }

    #[test]
    fn compile_intent_accepts_named_object_args_for_morpho_supply() {
        stable::init_storage();
        let template_id = "morpho-named-object-args";
        store_morpho_template_and_abi(template_id);
        let key = StrategyTemplateKey {
            protocol: "morpho-v1".to_string(),
            primitive: "lend_supply".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        };

        let intent = StrategyExecutionIntent {
            key: key.clone(),
            action_id: "enter_supply".to_string(),
            typed_params_json: serde_json::json!({
                "calls": [{
                    "args": morpho_supply_named_args_json(),
                    "value_wei": "0"
                }]
            })
            .to_string(),
        };

        let plan = compile_intent(&intent).expect("named-object morpho args should compile");
        assert_eq!(plan.key, key);
        assert_eq!(plan.calls.len(), 1);
        assert!(
            plan.calls[0].data.starts_with("0xa99aad89"),
            "calldata must start with supply selector, got {}",
            &plan.calls[0].data[..std::cmp::min(10, plan.calls[0].data.len())]
        );
    }

    #[test]
    fn compile_intent_named_and_positional_args_match_for_morpho_supply() {
        stable::init_storage();
        let template_id = "morpho-named-positional-parity";
        store_morpho_template_and_abi(template_id);
        let key = StrategyTemplateKey {
            protocol: "morpho-v1".to_string(),
            primitive: "lend_supply".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        };

        let positional_intent = StrategyExecutionIntent {
            key: key.clone(),
            action_id: "enter_supply".to_string(),
            typed_params_json: serde_json::json!({
                "calls": [{
                    "args": [
                        [
                            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                            "0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf",
                            "0x663E04CBb82e44A8544828C7C3e2f02820085f00",
                            "0x46415998764C29aB2a25CbeA6E5D2F226b40b5f0",
                            "860000000000000000"
                        ],
                        "1000000",
                        "0",
                        "0x1111111111111111111111111111111111111111",
                        "0x"
                    ],
                    "value_wei": "0"
                }]
            })
            .to_string(),
        };
        let named_intent = StrategyExecutionIntent {
            key,
            action_id: "enter_supply".to_string(),
            typed_params_json: serde_json::json!({
                "calls": [{
                    "args": morpho_supply_named_args_json(),
                    "value_wei": "0"
                }]
            })
            .to_string(),
        };

        let positional_plan = compile_intent(&positional_intent)
            .expect("legacy positional morpho args should still compile");
        let named_plan =
            compile_intent(&named_intent).expect("named-object morpho args should compile");

        assert_eq!(named_plan.calls, positional_plan.calls);
    }

    #[test]
    fn dry_run_compile_morpho_enter_supply_succeeds() {
        stable::init_storage();
        let template_id = "morpho-dry-run-regression";
        store_morpho_template_and_abi(template_id);
        let key = StrategyTemplateKey {
            protocol: "morpho-v1".to_string(),
            primitive: "lend_supply".to_string(),
            chain_id: 8453,
            template_id: template_id.to_string(),
        };

        let result = dry_run_compile(&key);
        assert!(
            result.is_ok(),
            "morpho enter_supply dry-run compile should pass: {result:?}"
        );
    }
}
