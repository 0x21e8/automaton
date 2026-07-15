use sha3::{Digest, Keccak256};

use crate::base_rpc;
use crate::cycles::ensure_sign_with_ecdsa_cycles;
use crate::state::{read_state, write_state};
use crate::types::{
    hex_encode_prefixed, FactoryError, ReleaseBroadcastConfig, ReleaseBroadcastFailure,
    ReleaseBroadcastRecord, ReleaseBroadcastStage, ReleaseSignatureRecord,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseBroadcastReceipt {
    pub release_tx_hash: String,
    pub release_broadcast_at: u64,
    pub record: ReleaseBroadcastRecord,
    pub confirmed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReproductionRelease {
    pub child_amount: u128,
    pub royalty_one_recipient: String,
    pub royalty_one_amount: u128,
    pub royalty_two_recipient: String,
    pub royalty_two_amount: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReleaseBroadcastError {
    pub(crate) record: ReleaseBroadcastRecord,
    pub(crate) source: Box<FactoryError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReleaseTransactionPlan {
    claim_id: String,
    recipient: String,
    escrow_contract_address: String,
    nonce: u64,
    config: ReleaseBroadcastConfig,
    calldata: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnsignedEip1559Transaction {
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: u64,
    max_fee_per_gas: u64,
    gas_limit: u64,
    to: [u8; 20],
    data: Vec<u8>,
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, FactoryError> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !trimmed.len().is_multiple_of(2) {
        return Err(FactoryError::InvalidAmount {
            value: value.to_string(),
        });
    }

    let mut bytes = Vec::with_capacity(trimmed.len() / 2);
    for chunk in trimmed.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0]).ok_or_else(|| FactoryError::InvalidAmount {
            value: value.to_string(),
        })?;
        let low = hex_value(chunk[1]).ok_or_else(|| FactoryError::InvalidAmount {
            value: value.to_string(),
        })?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], FactoryError> {
    let bytes = decode_hex(value)?;
    if bytes.len() != N {
        return Err(FactoryError::InvalidAmount {
            value: value.to_string(),
        });
    }
    let mut output = [0_u8; N];
    output.copy_from_slice(&bytes);
    Ok(output)
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = [0_u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn keccak_hex(bytes: &[u8]) -> String {
    let digest = keccak256(bytes);
    hex_encode_prefixed(&digest)
}

pub(crate) fn derive_evm_address_from_public_key(
    public_key: &[u8],
) -> Result<String, FactoryError> {
    use k256::ecdsa::VerifyingKey;

    let verifying_key = VerifyingKey::from_sec1_bytes(public_key).map_err(|error| {
        FactoryError::ManagementCallFailed {
            method: "ecdsa_public_key".to_string(),
            message: error.to_string(),
        }
    })?;
    let uncompressed = verifying_key.to_encoded_point(false);
    let encoded = uncompressed.as_bytes();
    let digest = keccak256(&encoded[1..]);
    Ok(hex_encode_prefixed(&digest[12..]))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn derive_child_evm_address_for_key_name(key_name: &str) -> String {
    let digest = Keccak256::digest(key_name.as_bytes());
    hex_encode_prefixed(&digest[12..32])
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn derive_child_evm_address(
    canister_id: &str,
    key_name: &str,
) -> Result<String, FactoryError> {
    use candid::Principal;
    use ic_cdk::management_canister::{
        ecdsa_public_key, EcdsaCurve, EcdsaKeyId, EcdsaPublicKeyArgs,
    };

    let canister_id =
        Principal::from_text(canister_id).map_err(|error| FactoryError::ManagementCallFailed {
            method: "parse_canister_id".to_string(),
            message: error.to_string(),
        })?;
    let response = ecdsa_public_key(&EcdsaPublicKeyArgs {
        canister_id: Some(canister_id),
        derivation_path: vec![b"evm".to_vec()],
        key_id: EcdsaKeyId {
            curve: EcdsaCurve::Secp256k1,
            name: key_name.to_string(),
        },
    })
    .await
    .map_err(|error| FactoryError::ManagementCallFailed {
        method: "ecdsa_public_key".to_string(),
        message: error.to_string(),
    })?;

    derive_evm_address_from_public_key(&response.public_key)
}

fn rlp_encode_length_prefix(len: usize, offset: u8, out: &mut Vec<u8>) {
    if len <= 55 {
        out.push(offset + len as u8);
        return;
    }

    let length_bytes = len.to_be_bytes();
    let first_non_zero = length_bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(length_bytes.len() - 1);
    let length_slice = &length_bytes[first_non_zero..];
    out.push(offset + 55 + length_slice.len() as u8);
    out.extend_from_slice(length_slice);
}

fn rlp_encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        out.push(bytes[0]);
        return;
    }

    rlp_encode_length_prefix(bytes.len(), 0x80, out);
    out.extend_from_slice(bytes);
}

fn rlp_encode_u64(value: u64, out: &mut Vec<u8>) {
    if value == 0 {
        out.push(0x80);
        return;
    }

    let bytes = value.to_be_bytes();
    let first_non_zero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    rlp_encode_bytes(&bytes[first_non_zero..], out);
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload_len: usize = items.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(payload_len + 9);
    rlp_encode_length_prefix(payload_len, 0xc0, &mut out);
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn encode_release_calldata(claim_id: &str, recipient: &str) -> Result<Vec<u8>, FactoryError> {
    let claim_id = decode_fixed_hex::<32>(claim_id)?;
    let recipient = decode_fixed_hex::<20>(recipient)?;
    let selector = &keccak256(b"release(bytes32,address)")[..4];

    let mut encoded = Vec::with_capacity(4 + 64);
    encoded.extend_from_slice(selector);
    encoded.extend_from_slice(&claim_id);

    let mut recipient_word = [0_u8; 32];
    recipient_word[12..].copy_from_slice(&recipient);
    encoded.extend_from_slice(&recipient_word);
    Ok(encoded)
}

fn encode_refund_calldata(claim_id: &str) -> Result<Vec<u8>, FactoryError> {
    let claim_id = decode_fixed_hex::<32>(claim_id)?;
    let selector = &keccak256(b"refund(bytes32)")[..4];
    let mut encoded = Vec::with_capacity(4 + 32);
    encoded.extend_from_slice(selector);
    encoded.extend_from_slice(&claim_id);
    Ok(encoded)
}

fn encode_address_word(value: &str) -> Result<[u8; 32], FactoryError> {
    let address = decode_fixed_hex::<20>(value)?;
    let mut word = [0_u8; 32];
    word[12..].copy_from_slice(&address);
    Ok(word)
}

fn encode_u128_word(value: u128) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

fn encode_reproduction_release_calldata(
    claim_id: &str,
    child: &str,
    release: &ReproductionRelease,
) -> Result<Vec<u8>, FactoryError> {
    let claim_id = decode_fixed_hex::<32>(claim_id)?;
    let selector =
        &keccak256(b"releaseReproduction(bytes32,address,uint256,address,uint256,address,uint256)")
            [..4];
    let mut encoded = Vec::with_capacity(4 + 7 * 32);
    encoded.extend_from_slice(selector);
    encoded.extend_from_slice(&claim_id);
    encoded.extend_from_slice(&encode_address_word(child)?);
    encoded.extend_from_slice(&encode_u128_word(release.child_amount));
    encoded.extend_from_slice(&encode_address_word(&release.royalty_one_recipient)?);
    encoded.extend_from_slice(&encode_u128_word(release.royalty_one_amount));
    encoded.extend_from_slice(&encode_address_word(&release.royalty_two_recipient)?);
    encoded.extend_from_slice(&encode_u128_word(release.royalty_two_amount));
    Ok(encoded)
}

fn build_release_plan(
    claim_id: &str,
    recipient: &str,
    escrow_contract_address: &str,
    nonce: u64,
    config: ReleaseBroadcastConfig,
) -> Result<ReleaseTransactionPlan, FactoryError> {
    Ok(ReleaseTransactionPlan {
        claim_id: claim_id.to_string(),
        recipient: recipient.to_string(),
        escrow_contract_address: escrow_contract_address.to_string(),
        nonce,
        calldata: encode_release_calldata(claim_id, recipient)?,
        config,
    })
}

fn build_refund_plan(
    claim_id: &str,
    recipient: &str,
    escrow_contract_address: &str,
    nonce: u64,
    config: ReleaseBroadcastConfig,
) -> Result<ReleaseTransactionPlan, FactoryError> {
    Ok(ReleaseTransactionPlan {
        claim_id: claim_id.to_string(),
        recipient: recipient.to_string(),
        escrow_contract_address: escrow_contract_address.to_string(),
        nonce,
        calldata: encode_refund_calldata(claim_id)?,
        config,
    })
}

fn build_reproduction_release_plan(
    claim_id: &str,
    recipient: &str,
    escrow_contract_address: &str,
    nonce: u64,
    config: ReleaseBroadcastConfig,
    release: &ReproductionRelease,
) -> Result<ReleaseTransactionPlan, FactoryError> {
    Ok(ReleaseTransactionPlan {
        claim_id: claim_id.to_string(),
        recipient: recipient.to_string(),
        escrow_contract_address: escrow_contract_address.to_string(),
        nonce,
        calldata: encode_reproduction_release_calldata(claim_id, recipient, release)?,
        config,
    })
}

fn build_release_broadcast_record(plan: &ReleaseTransactionPlan) -> ReleaseBroadcastRecord {
    ReleaseBroadcastRecord {
        claim_id: plan.claim_id.clone(),
        recipient: plan.recipient.clone(),
        escrow_contract_address: plan.escrow_contract_address.clone(),
        nonce: plan.nonce,
        chain_id: plan.config.chain_id,
        max_priority_fee_per_gas: plan.config.max_priority_fee_per_gas,
        max_fee_per_gas: plan.config.max_fee_per_gas,
        gas_limit: plan.config.gas_limit,
        calldata_hex: hex_encode_prefixed(&plan.calldata),
        signing_payload_hash: None,
        signature: None,
        raw_transaction_hash: None,
        rpc_tx_hash: None,
        broadcast_at: None,
        last_error: None,
    }
}

fn build_unsigned_release_transaction(
    plan: &ReleaseTransactionPlan,
) -> Result<UnsignedEip1559Transaction, FactoryError> {
    Ok(UnsignedEip1559Transaction {
        chain_id: plan.config.chain_id,
        nonce: plan.nonce,
        max_priority_fee_per_gas: plan.config.max_priority_fee_per_gas,
        max_fee_per_gas: plan.config.max_fee_per_gas,
        gas_limit: plan.config.gas_limit,
        to: decode_fixed_hex::<20>(&plan.escrow_contract_address)?,
        data: plan.calldata.clone(),
    })
}

fn rlp_encode_eip1559_base_fields(tx: &UnsignedEip1559Transaction) -> Vec<Vec<u8>> {
    let encode_u64 = |value| {
        let mut buf = Vec::new();
        rlp_encode_u64(value, &mut buf);
        buf
    };
    let encode_bytes = |bytes: &[u8]| {
        let mut buf = Vec::new();
        rlp_encode_bytes(bytes, &mut buf);
        buf
    };

    vec![
        encode_u64(tx.chain_id),
        encode_u64(tx.nonce),
        encode_u64(tx.max_priority_fee_per_gas),
        encode_u64(tx.max_fee_per_gas),
        encode_u64(tx.gas_limit),
        encode_bytes(&tx.to),
        encode_u64(0), // value (always zero for release transactions)
        encode_bytes(&tx.data),
        vec![0xc0], // empty access list
    ]
}

fn eip1559_envelope(payload: &[u8]) -> Vec<u8> {
    let mut envelope = Vec::with_capacity(payload.len() + 1);
    envelope.push(0x02); // EIP-1559 type byte
    envelope.extend_from_slice(payload);
    envelope
}

fn build_signing_payload(tx: &UnsignedEip1559Transaction) -> Vec<u8> {
    let fields = rlp_encode_eip1559_base_fields(tx);
    eip1559_envelope(&rlp_encode_list(&fields))
}

fn build_signed_raw_transaction(
    tx: &UnsignedEip1559Transaction,
    y_parity: bool,
    signature: &[u8],
) -> Result<Vec<u8>, FactoryError> {
    if signature.len() != 64 {
        return Err(FactoryError::ManagementCallFailed {
            method: "sign_with_ecdsa".to_string(),
            message: "signature must be 64 bytes".to_string(),
        });
    }

    let mut fields = rlp_encode_eip1559_base_fields(tx);

    let mut y_parity_bytes = Vec::new();
    rlp_encode_u64(u64::from(y_parity), &mut y_parity_bytes);
    let mut r_bytes = Vec::new();
    rlp_encode_bytes(&signature[..32], &mut r_bytes);
    let mut s_bytes = Vec::new();
    rlp_encode_bytes(&signature[32..], &mut s_bytes);

    fields.push(y_parity_bytes);
    fields.push(r_bytes);
    fields.push(s_bytes);

    Ok(eip1559_envelope(&rlp_encode_list(&fields)))
}

fn signature_record(
    y_parity: bool,
    signature: &[u8],
) -> Result<ReleaseSignatureRecord, FactoryError> {
    if signature.len() != 64 {
        return Err(FactoryError::ManagementCallFailed {
            method: "sign_with_ecdsa".to_string(),
            message: "signature must be 64 bytes".to_string(),
        });
    }

    Ok(ReleaseSignatureRecord {
        y_parity,
        r: hex_encode_prefixed(&signature[..32]),
        s: hex_encode_prefixed(&signature[32..]),
    })
}

fn release_broadcast_error(
    stage: ReleaseBroadcastStage,
    mut record: ReleaseBroadcastRecord,
    error: FactoryError,
    now_ms: u64,
) -> ReleaseBroadcastError {
    let (rpc_category, rpc_code, rpc_endpoint, message) = match &error {
        FactoryError::RpcRequestFailed {
            category,
            code,
            endpoint,
            message,
            ..
        } => (
            Some(category.clone()),
            *code,
            Some(endpoint.clone()),
            message.clone(),
        ),
        FactoryError::ManagementCallFailed { message, .. } => (None, None, None, message.clone()),
        other => (None, None, None, other.to_string()),
    };

    record.last_error = Some(ReleaseBroadcastFailure {
        stage,
        message,
        rpc_category,
        rpc_code,
        rpc_endpoint,
        occurred_at: now_ms,
    });

    ReleaseBroadcastError {
        record,
        source: Box::new(error),
    }
}

pub fn derive_factory_evm_address_from_public_key(
    public_key: &[u8],
) -> Result<String, FactoryError> {
    if let Some(address) = read_state(|state| state.factory_evm_address.clone()) {
        return Ok(address);
    }

    let address = derive_evm_address_from_public_key(public_key)?;
    write_state(|state| {
        state.factory_evm_address = Some(address.clone());
        state.factory_evm_address_derived_at = Some({
            #[cfg(target_arch = "wasm32")]
            {
                ic_cdk::api::time() / 1_000_000
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                0
            }
        });
    });
    Ok(address)
}

#[cfg(target_arch = "wasm32")]
fn derive_signed_recid(
    prehash: &[u8],
    signature: &[u8],
    public_key: &[u8],
) -> Result<bool, FactoryError> {
    use core::convert::TryFrom;
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let signature =
        Signature::try_from(signature).map_err(|error| FactoryError::ManagementCallFailed {
            method: "sign_with_ecdsa".to_string(),
            message: error.to_string(),
        })?;
    let expected_key = VerifyingKey::from_sec1_bytes(public_key).map_err(|error| {
        FactoryError::ManagementCallFailed {
            method: "ecdsa_public_key".to_string(),
            message: error.to_string(),
        }
    })?;

    for candidate in 0_u8..=3 {
        let recid = RecoveryId::try_from(candidate).expect("candidate in range");
        if let Ok(recovered) = VerifyingKey::recover_from_prehash(prehash, &signature, recid) {
            if recovered == expected_key {
                return Ok(recid.is_y_odd());
            }
        }
    }

    Err(FactoryError::ManagementCallFailed {
        method: "sign_with_ecdsa".to_string(),
        message: "unable to recover ECDSA recovery id".to_string(),
    })
}

#[cfg(target_arch = "wasm32")]
fn factory_public_key_argument(key_name: &str) -> ic_cdk::management_canister::EcdsaPublicKeyArgs {
    use ic_cdk::management_canister::{EcdsaCurve, EcdsaKeyId, EcdsaPublicKeyArgs};

    EcdsaPublicKeyArgs {
        canister_id: None,
        derivation_path: Vec::new(),
        key_id: EcdsaKeyId {
            curve: EcdsaCurve::Secp256k1,
            name: key_name.to_string(),
        },
    }
}

fn factory_signing_argument(
    message_hash: Vec<u8>,
    key_name: &str,
) -> ic_cdk::management_canister::SignWithEcdsaArgs {
    use ic_cdk::management_canister::{EcdsaCurve, EcdsaKeyId, SignWithEcdsaArgs};

    SignWithEcdsaArgs {
        message_hash,
        derivation_path: Vec::new(),
        key_id: EcdsaKeyId {
            curve: EcdsaCurve::Secp256k1,
            name: key_name.to_string(),
        },
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn derive_factory_evm_address() -> Result<String, FactoryError> {
    use ic_cdk::management_canister::ecdsa_public_key;

    if let Some(address) = read_state(|state| state.factory_evm_address.clone()) {
        return Ok(address);
    }

    let key_name = read_state(|state| state.release_broadcast_config.ecdsa_key_name.clone());
    let response = ecdsa_public_key(&factory_public_key_argument(&key_name))
        .await
        .map_err(|error| FactoryError::ManagementCallFailed {
            method: "ecdsa_public_key".to_string(),
            message: error.to_string(),
        })?;
    let address = derive_factory_evm_address_from_public_key(&response.public_key)?;
    Ok(address)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn derive_factory_evm_address_from_bytes(public_key: &[u8]) -> Result<String, FactoryError> {
    derive_factory_evm_address_from_public_key(public_key)
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn broadcast_release_transaction(
    claim_id: &str,
    recipient: &str,
    base_rpc_endpoints: &[String],
    escrow_contract_address: &str,
    nonce: u64,
    now_ms: u64,
    config: &ReleaseBroadcastConfig,
    reproduction: Option<&ReproductionRelease>,
    refund: bool,
) -> Result<ReleaseBroadcastReceipt, ReleaseBroadcastError> {
    use ic_cdk::management_canister::{ecdsa_public_key, sign_with_ecdsa};

    let plan_result = if refund {
        build_refund_plan(
            claim_id,
            recipient,
            escrow_contract_address,
            nonce,
            config.clone(),
        )
    } else {
        match reproduction {
            Some(release) => build_reproduction_release_plan(
                claim_id,
                recipient,
                escrow_contract_address,
                nonce,
                config.clone(),
                release,
            ),
            None => build_release_plan(
                claim_id,
                recipient,
                escrow_contract_address,
                nonce,
                config.clone(),
            ),
        }
    };
    let plan = match plan_result {
        Ok(plan) => plan,
        Err(error) => {
            return Err(release_broadcast_error(
                ReleaseBroadcastStage::CalldataEncoding,
                ReleaseBroadcastRecord {
                    claim_id: claim_id.to_string(),
                    recipient: recipient.to_string(),
                    escrow_contract_address: escrow_contract_address.to_string(),
                    nonce,
                    chain_id: config.chain_id,
                    max_priority_fee_per_gas: config.max_priority_fee_per_gas,
                    max_fee_per_gas: config.max_fee_per_gas,
                    gas_limit: config.gas_limit,
                    calldata_hex: "0x".to_string(),
                    signing_payload_hash: None,
                    signature: None,
                    raw_transaction_hash: None,
                    rpc_tx_hash: None,
                    broadcast_at: None,
                    last_error: None,
                },
                error,
                now_ms,
            ));
        }
    };
    let mut record = build_release_broadcast_record(&plan);
    let unsigned_tx = build_unsigned_release_transaction(&plan).map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::SigningPayloadConstruction,
            record.clone(),
            error,
            now_ms,
        )
    })?;
    let signing_payload = build_signing_payload(&unsigned_tx);
    let prehash = keccak256(&signing_payload);
    record.signing_payload_hash = Some(keccak_hex(&signing_payload));

    let response = ecdsa_public_key(&factory_public_key_argument(&config.ecdsa_key_name))
        .await
        .map_err(|error| {
            release_broadcast_error(
                ReleaseBroadcastStage::PublicKeyLookup,
                record.clone(),
                FactoryError::ManagementCallFailed {
                    method: "ecdsa_public_key".to_string(),
                    message: error.to_string(),
                },
                now_ms,
            )
        })?;

    let signing_request = factory_signing_argument(prehash.to_vec(), &config.ecdsa_key_name);
    ensure_sign_with_ecdsa_cycles("sign_with_ecdsa", &signing_request).map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::Signing,
            record.clone(),
            error,
            now_ms,
        )
    })?;

    let signing_result = sign_with_ecdsa(&signing_request).await.map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::Signing,
            record.clone(),
            FactoryError::ManagementCallFailed {
                method: "sign_with_ecdsa".to_string(),
                message: error.to_string(),
            },
            now_ms,
        )
    })?;

    let y_parity = derive_signed_recid(&prehash, &signing_result.signature, &response.public_key)
        .map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::SignatureRecovery,
            record.clone(),
            error,
            now_ms,
        )
    })?;
    record.signature = Some(
        signature_record(y_parity, &signing_result.signature).map_err(|error| {
            release_broadcast_error(
                ReleaseBroadcastStage::SignatureRecovery,
                record.clone(),
                error,
                now_ms,
            )
        })?,
    );

    let raw_tx = build_signed_raw_transaction(&unsigned_tx, y_parity, &signing_result.signature)
        .map_err(|error| {
            release_broadcast_error(
                ReleaseBroadcastStage::RawTransactionConstruction,
                record.clone(),
                error,
                now_ms,
            )
        })?;
    let raw_tx_hex = hex_encode_prefixed(&raw_tx);
    record.raw_transaction_hash = Some(keccak_hex(&raw_tx));

    let rpc_hash = base_rpc::eth_send_raw_transaction(base_rpc_endpoints, &raw_tx_hex)
        .await
        .map_err(|error| {
            release_broadcast_error(
                ReleaseBroadcastStage::RpcBroadcast,
                record.clone(),
                error,
                now_ms,
            )
        })?;
    record.rpc_tx_hash = Some(rpc_hash.clone());
    record.broadcast_at = Some(now_ms);

    let confirmed =
        match base_rpc::eth_get_transaction_receipt_status(base_rpc_endpoints, &rpc_hash).await {
            Ok(Some(true)) => true,
            Ok(Some(false)) => {
                return Err(release_broadcast_error(
                    ReleaseBroadcastStage::ReceiptConfirmation,
                    record,
                    FactoryError::ManagementCallFailed {
                        method: "eth_getTransactionReceipt".to_string(),
                        message: "release transaction reverted".to_string(),
                    },
                    now_ms,
                ));
            }
            Ok(None) => false,
            Err(error) => {
                return Err(release_broadcast_error(
                    ReleaseBroadcastStage::ReceiptConfirmation,
                    record,
                    error,
                    now_ms,
                ));
            }
        };

    Ok(ReleaseBroadcastReceipt {
        release_tx_hash: rpc_hash,
        release_broadcast_at: now_ms,
        record,
        confirmed,
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::result_large_err)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn broadcast_release_transaction(
    claim_id: &str,
    recipient: &str,
    base_rpc_endpoints: &[String],
    escrow_contract_address: &str,
    nonce: u64,
    now_ms: u64,
    config: &ReleaseBroadcastConfig,
    reproduction: Option<&ReproductionRelease>,
    refund: bool,
) -> Result<ReleaseBroadcastReceipt, ReleaseBroadcastError> {
    let plan = if refund {
        build_refund_plan(
            claim_id,
            recipient,
            escrow_contract_address,
            nonce,
            config.clone(),
        )
    } else {
        match reproduction {
            Some(release) => build_reproduction_release_plan(
                claim_id,
                recipient,
                escrow_contract_address,
                nonce,
                config.clone(),
                release,
            ),
            None => build_release_plan(
                claim_id,
                recipient,
                escrow_contract_address,
                nonce,
                config.clone(),
            ),
        }
    }
    .map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::CalldataEncoding,
            ReleaseBroadcastRecord {
                claim_id: claim_id.to_string(),
                recipient: recipient.to_string(),
                escrow_contract_address: escrow_contract_address.to_string(),
                nonce,
                chain_id: config.chain_id,
                max_priority_fee_per_gas: config.max_priority_fee_per_gas,
                max_fee_per_gas: config.max_fee_per_gas,
                gas_limit: config.gas_limit,
                calldata_hex: "0x".to_string(),
                signing_payload_hash: None,
                signature: None,
                raw_transaction_hash: None,
                rpc_tx_hash: None,
                broadcast_at: None,
                last_error: None,
            },
            error,
            now_ms,
        )
    })?;
    let mut record = build_release_broadcast_record(&plan);
    let unsigned_tx = build_unsigned_release_transaction(&plan).map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::SigningPayloadConstruction,
            record.clone(),
            error,
            now_ms,
        )
    })?;
    let signing_payload = build_signing_payload(&unsigned_tx);
    record.signing_payload_hash = Some(keccak_hex(&signing_payload));
    let signing_request =
        factory_signing_argument(keccak256(&signing_payload).to_vec(), &config.ecdsa_key_name);

    ensure_sign_with_ecdsa_cycles("sign_with_ecdsa", &signing_request).map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::Signing,
            record.clone(),
            error,
            now_ms,
        )
    })?;

    let stub_signature = [0x11_u8; 64];
    record.signature = Some(signature_record(false, &stub_signature).map_err(|error| {
        release_broadcast_error(
            ReleaseBroadcastStage::SignatureRecovery,
            record.clone(),
            error,
            now_ms,
        )
    })?);

    let raw_tx =
        build_signed_raw_transaction(&unsigned_tx, false, &stub_signature).map_err(|error| {
            release_broadcast_error(
                ReleaseBroadcastStage::RawTransactionConstruction,
                record.clone(),
                error,
                now_ms,
            )
        })?;
    let raw_tx_hex = hex_encode_prefixed(&raw_tx);
    let local_tx_hash = keccak_hex(&raw_tx);
    record.raw_transaction_hash = Some(local_tx_hash.clone());

    let rpc_hash =
        base_rpc::eth_send_raw_transaction(base_rpc_endpoints, &raw_tx_hex, &local_tx_hash)
            .map_err(|error| {
                release_broadcast_error(
                    ReleaseBroadcastStage::RpcBroadcast,
                    record.clone(),
                    error,
                    now_ms,
                )
            })?;
    record.rpc_tx_hash = Some(rpc_hash.clone());
    record.broadcast_at = Some(now_ms);

    Ok(ReleaseBroadcastReceipt {
        release_tx_hash: rpc_hash,
        release_broadcast_at: now_ms,
        record,
        confirmed: true,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        broadcast_release_transaction, build_release_broadcast_record, build_release_plan,
        build_signing_payload, build_unsigned_release_transaction, ReleaseBroadcastStage,
    };
    use crate::state::{restore_state, set_mock_canister_balance, snapshot_state};
    use crate::{FactoryError, FactoryStateSnapshot, ReleaseBroadcastConfig};

    #[test]
    fn configured_fees_flow_into_unsigned_release_transactions() {
        restore_state(FactoryStateSnapshot::default());
        crate::state::write_state(|state| {
            state.release_broadcast_config = ReleaseBroadcastConfig {
                chain_id: 31_337,
                max_priority_fee_per_gas: 2,
                max_fee_per_gas: 9,
                gas_limit: 555_000,
                ecdsa_key_name: "test_key_1".to_string(),
            };
        });

        let config = snapshot_state().release_broadcast_config;
        let plan = build_release_plan(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            "0x3333333333333333333333333333333333333333",
            7,
            config.clone(),
        )
        .expect("plan should build");
        let record = build_release_broadcast_record(&plan);
        let tx = build_unsigned_release_transaction(&plan).expect("transaction should build");
        let signing_payload = build_signing_payload(&tx);

        assert_eq!(record.chain_id, 31_337);
        assert_eq!(record.max_priority_fee_per_gas, 2);
        assert_eq!(record.max_fee_per_gas, 9);
        assert_eq!(record.gas_limit, 555_000);
        assert!(!signing_payload.is_empty());
    }

    #[test]
    fn records_rpc_failure_context_for_release_broadcasts() {
        restore_state(FactoryStateSnapshot::default());

        let error = broadcast_release_transaction(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            &["mock://error/rate-limit".to_string()],
            "0x3333333333333333333333333333333333333333",
            99,
            1_234,
            &ReleaseBroadcastConfig::default(),
            None,
            false,
        )
        .expect_err("broadcast should fail");

        assert_eq!(
            error.record.last_error.as_ref().map(|entry| &entry.stage),
            Some(&ReleaseBroadcastStage::RpcBroadcast)
        );
        assert_eq!(
            error
                .record
                .last_error
                .as_ref()
                .and_then(|entry| entry.rpc_code),
            Some(429)
        );
        assert_eq!(error.record.nonce, 99);
        assert!(error.record.signing_payload_hash.is_some());
        assert!(error.record.signature.is_some());
    }

    #[test]
    fn records_successful_release_broadcast_context() {
        restore_state(FactoryStateSnapshot::default());

        let receipt = broadcast_release_transaction(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            &["mock://success".to_string()],
            "0x3333333333333333333333333333333333333333",
            5,
            9_876,
            &ReleaseBroadcastConfig::default(),
            None,
            false,
        )
        .expect("broadcast should succeed");

        assert_eq!(receipt.record.nonce, 5);
        assert_eq!(receipt.record.broadcast_at, Some(9_876));
        assert_eq!(
            receipt.record.rpc_tx_hash.as_deref(),
            Some(receipt.release_tx_hash.as_str())
        );
        assert!(receipt.record.raw_transaction_hash.is_some());
    }

    #[test]
    fn rejects_release_broadcast_when_signing_cycles_are_not_affordable() {
        restore_state(FactoryStateSnapshot::default());
        set_mock_canister_balance(0);

        let error = broadcast_release_transaction(
            "0x1111111111111111111111111111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            &["mock://success".to_string()],
            "0x3333333333333333333333333333333333333333",
            5,
            9_876,
            &ReleaseBroadcastConfig::default(),
            None,
            false,
        )
        .expect_err("broadcast should fail before signing");

        assert_eq!(
            error.record.last_error.as_ref().map(|entry| &entry.stage),
            Some(&ReleaseBroadcastStage::Signing)
        );
        assert!(matches!(
            *error.source,
            FactoryError::InsufficientCyclesForOperation { ref operation, .. }
                if operation == "sign_with_ecdsa"
        ));
        set_mock_canister_balance(u128::MAX);
    }
}
