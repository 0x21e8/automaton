import type {
  CreateSpawnSessionRequest,
  CreateSpawnSessionResponse,
  FactoryStewardCommand,
  FactoryStewardProofTemplate,
  RefundSpawnResponse,
  RetrySpawnResponse,
  SpawnSessionDetail
} from "@ic-automaton/shared";

import { requestIndexerJson } from "./indexer";

export async function createSpawnSession(
  request: CreateSpawnSessionRequest,
  signal?: AbortSignal
): Promise<CreateSpawnSessionResponse> {
  return requestIndexerJson<CreateSpawnSessionResponse>("/api/spawn-sessions", {
    method: "POST",
    body: request,
    signal
  });
}

export async function fetchSpawnSessionDetail(
  sessionId: string,
  signal?: AbortSignal
): Promise<SpawnSessionDetail> {
  return requestIndexerJson<SpawnSessionDetail>(`/api/spawn-sessions/${sessionId}`, {
    signal
  });
}

export async function retrySpawnSession(
  sessionId: string,
  walletAddress: string,
  walletRequest: <T = unknown>(args: { method: string; params?: unknown[] }) => Promise<T>,
  signal?: AbortSignal
): Promise<RetrySpawnResponse> {
  const command: FactoryStewardCommand = { retrySpawnSession: { sessionId } };
  const request = await signFactoryCommand(sessionId, command, walletAddress, walletRequest, signal);
  try {
    return await requestIndexerJson<RetrySpawnResponse>(`/api/spawn-sessions/${sessionId}/retry`, {
      method: "POST", body: request, signal
    });
  } catch (error) {
    const reconciled = await reconcileLostFactoryResponse(sessionId, command, request.proof.nonce, signal);
    if (reconciled.advanced) return { session: reconciled.detail.session };
    throw error;
  }
}

export async function refundSpawnSession(
  sessionId: string,
  walletAddress: string,
  walletRequest: <T = unknown>(args: { method: string; params?: unknown[] }) => Promise<T>,
  signal?: AbortSignal
): Promise<RefundSpawnResponse> {
  const command: FactoryStewardCommand = { claimSpawnRefund: { sessionId } };
  const request = await signFactoryCommand(sessionId, command, walletAddress, walletRequest, signal);
  try {
    return await requestIndexerJson<RefundSpawnResponse>(`/api/spawn-sessions/${sessionId}/refund`, {
      method: "POST", body: request, signal
    });
  } catch (error) {
    const reconciled = await reconcileLostFactoryResponse(sessionId, command, request.proof.nonce, signal);
    if (reconciled.advanced) {
      throw new Error(
        reconciled.detail.session.paymentStatus === "refunded"
          ? "Factory accepted and completed the signed refund command; refresh for authoritative refund receipt details."
          : "Factory accepted the signed refund command; confirmation is still pending."
      );
    }
    throw error;
  }
}

async function reconcileLostFactoryResponse(
  sessionId: string,
  command: FactoryStewardCommand,
  submittedNonce: string,
  signal?: AbortSignal
) {
  const [detail, nextTemplate] = await Promise.all([
    fetchSpawnSessionDetail(sessionId, signal),
    requestIndexerJson<FactoryStewardProofTemplate>(
      `/api/spawn-sessions/${sessionId}/steward-command`,
      { method: "POST", body: { command }, signal }
    )
  ]);
  return { detail, advanced: BigInt(nextTemplate.nonce) > BigInt(submittedNonce) };
}

function utf8Hex(value: string): string {
  return `0x${Array.from(new TextEncoder().encode(value), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

async function signFactoryCommand(
  sessionId: string,
  command: FactoryStewardCommand,
  walletAddress: string,
  walletRequest: <T = unknown>(args: { method: string; params?: unknown[] }) => Promise<T>,
  signal?: AbortSignal
) {
  const template = await requestIndexerJson<FactoryStewardProofTemplate>(
    `/api/spawn-sessions/${sessionId}/steward-command`,
    { method: "POST", body: { command }, signal }
  );
  if (template.address.toLowerCase() !== walletAddress.toLowerCase()) {
    throw new Error(`Connected wallet ${walletAddress} is not the session steward ${template.address}.`);
  }
  if (BigInt(template.expiresAtNs) <= BigInt(Date.now()) * 1_000_000n) {
    throw new Error("Factory steward proof template expired before signing.");
  }
  const signature = await walletRequest<string>({
    method: "personal_sign",
    params: [utf8Hex(template.signingPayload), walletAddress]
  });
  return {
    command,
    proof: {
      chainId: template.chainId,
      address: template.address,
      commandHash: template.commandHash,
      nonce: template.nonce,
      expiresAtNs: template.expiresAtNs,
      signature
    }
  };
}
