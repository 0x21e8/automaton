import type {
  PlaygroundMetadata,
  SpawnPaymentInstructions,
  SpawnSession
} from "@ic-automaton/shared";

import type { WalletTransport } from "./wallet-transport";
import {
  bigintToHex,
  encodeErc20ApproveData,
  encodeEscrowDepositData,
  resolveSpawnChainId,
  resolveSpawnChainMetadata,
  resolveSpawnUsdcContractAddress
} from "./wallet-transaction-helpers";

export interface SpawnPaymentWalletState {
  address: string | null;
  chainId: number | null;
}

export interface SpawnPaymentAvailability {
  canSubmit: boolean;
  disabledReason: string | null;
  expectedChainId: number | null;
}

export interface SpawnPaymentExecutionResult {
  approvalTxHash: string;
  paymentTxHash: string;
  pendingReceipts: Array<"approval" | "deposit">;
}

interface JsonRpcBlockHeader {
  hash?: unknown;
  number?: unknown;
}

const UNKNOWN_CHAIN_ERROR_CODE = 4902;
const USER_REJECTED_REQUEST_ERROR_CODE = 4001;
const RECEIPT_POLL_INTERVAL_MS = 1_000;
const RECEIPT_TIMEOUT_MS = 30_000;

function parseRawTokenAmount(value: string): bigint | null {
  const normalized = value.trim();

  if (!/^\d+$/.test(normalized)) {
    return null;
  }

  return BigInt(normalized);
}

class SpawnPaymentError extends Error {
  readonly kind:
    | "chain_add_rejected"
    | "chain_add_failed"
    | "chain_switch_rejected"
    | "insufficient_eth"
    | "insufficient_usdc"
    | "quote_expired"
    | "stale_network_config";

  constructor(kind: SpawnPaymentError["kind"], message: string) {
    super(message);
    this.kind = kind;
  }
}

function extractProviderErrorMessage(error: unknown): string | null {
  if (typeof error !== "object" || error === null) {
    return null;
  }

  if ("message" in error && typeof error.message === "string" && error.message.trim() !== "") {
    return error.message.trim();
  }

  if (
    "data" in error &&
    typeof error.data === "object" &&
    error.data !== null &&
    "message" in error.data &&
    typeof error.data.message === "string" &&
    error.data.message.trim() !== ""
  ) {
    return error.data.message.trim();
  }

  return null;
}

function toHexChainId(chainId: number): string {
  return `0x${chainId.toString(16)}`;
}

function isProviderErrorWithCode(error: unknown, code: number) {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === code
  );
}

async function requestJsonRpcResult<T>(
  rpcUrl: string,
  method: string,
  params: unknown[]
): Promise<T> {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: {
      "content-type": "application/json"
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now(),
      method,
      params
    })
  });

  const payload = (await response.json()) as {
    error?: {
      message?: unknown;
    };
    result?: unknown;
  };

  if (!response.ok || payload.error) {
    throw new Error(
      typeof payload.error?.message === "string"
        ? payload.error.message
        : `${method} failed.`
    );
  }

  return payload.result as T;
}

async function requestTransactionReceipt(
  txHash: string,
  transport: WalletTransport,
  rpcUrl: string | null
): Promise<
  | {
      status?: unknown;
    }
  | null
> {
  let transportReceipt:
    | {
        status?: unknown;
      }
    | null
    | undefined;

  try {
    transportReceipt = await transport.request<
      | {
          status?: unknown;
        }
      | null
    >({
      method: "eth_getTransactionReceipt",
      params: [txHash]
    });
  } catch {}

  if (transportReceipt !== undefined && transportReceipt !== null) {
    return transportReceipt;
  }

  if (rpcUrl === null) {
    return transportReceipt ?? null;
  }

  try {
    return await requestJsonRpcResult<
      | {
          status?: unknown;
        }
      | null
    >(rpcUrl, "eth_getTransactionReceipt", [txHash]);
  } catch {
    return transportReceipt ?? null;
  }
}

async function waitForTransactionReceipt(
  txHash: string,
  transport: WalletTransport,
  chain: SpawnSession["chain"],
  label: string,
  playgroundMetadata: PlaygroundMetadata | null,
  env: Record<string, string | undefined>
): Promise<"confirmed" | "pending"> {
  const rpcUrl = resolveSpawnChainMetadata(chain, playgroundMetadata, env)?.rpcUrl ?? null;

  const deadline = Date.now() + RECEIPT_TIMEOUT_MS;

  while (Date.now() < deadline) {
    const receipt = await requestTransactionReceipt(txHash, transport, rpcUrl);

    if (receipt !== null) {
      if (typeof receipt.status === "string" && receipt.status.toLowerCase() === "0x0") {
        throw new Error(`${label} transaction reverted on-chain.`);
      }

      return "confirmed";
    }

    await new Promise((resolve) => setTimeout(resolve, RECEIPT_POLL_INTERVAL_MS));
  }

  return "pending";
}

async function requestWalletChainSwitch(
  chainId: number,
  transport: WalletTransport
) {
  await transport.request({
    method: "wallet_switchEthereumChain",
    params: [{ chainId: toHexChainId(chainId) }]
  });
}

function normalizeHexQuantity(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }

  const normalized = value.trim().toLowerCase();
  return /^0x[0-9a-f]+$/.test(normalized) ? normalized : null;
}

function normalizeBlockHeader(
  value: unknown
): { hash: string; number: string } | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }

  const hash = "hash" in value ? normalizeHexQuantity(value.hash) : null;
  const number = "number" in value ? normalizeHexQuantity(value.number) : null;

  if (hash === null || number === null) {
    return null;
  }

  return {
    hash,
    number
  };
}

function isEmptyContractCode(value: unknown) {
  return typeof value === "string" && /^0x0*$/.test(value.trim().toLowerCase());
}

async function requestWalletRpcResult<T>(
  transport: WalletTransport,
  method: string,
  params: unknown[]
): Promise<T | null> {
  try {
    return await transport.request<T>({
      method,
      params
    });
  } catch {
    return null;
  }
}

async function assertWalletTargetsActivePlaygroundNetwork(
  payment: SpawnPaymentInstructions,
  transport: WalletTransport,
  tokenAddress: string,
  playgroundMetadata: PlaygroundMetadata | null,
  env: Record<string, string | undefined>
) {
  const chainMetadata = resolveSpawnChainMetadata(payment.chain, playgroundMetadata, env);
  const rpcUrl = chainMetadata?.rpcUrl ?? null;

  if (rpcUrl !== null) {
    const trustedLatestBlock = normalizeBlockHeader(
      await requestJsonRpcResult<JsonRpcBlockHeader>(rpcUrl, "eth_getBlockByNumber", [
        "latest",
        false
      ])
    );

    if (trustedLatestBlock !== null) {
      const walletBlock = normalizeBlockHeader(
        await requestWalletRpcResult<JsonRpcBlockHeader>(
          transport,
          "eth_getBlockByNumber",
          [trustedLatestBlock.number, false]
        )
      );

      if (walletBlock !== null && walletBlock.hash !== trustedLatestBlock.hash) {
        throw new SpawnPaymentError(
          "stale_network_config",
          `Connected wallet is pointed at a different Automaton Playground network than chain ${chainMetadata?.chainId ?? "the expected chain"}. Remove the existing playground network from the wallet, add it again, and retry the payment.`
        );
      }
    }
  }

  const [walletTokenCode, walletEscrowCode] = await Promise.all([
    requestWalletRpcResult<string>(transport, "eth_getCode", [tokenAddress, "latest"]),
    requestWalletRpcResult<string>(transport, "eth_getCode", [payment.paymentAddress, "latest"])
  ]);

  if (isEmptyContractCode(walletTokenCode) || isEmptyContractCode(walletEscrowCode)) {
    throw new SpawnPaymentError(
      "stale_network_config",
      `Connected wallet is missing the live playground contracts for chain ${chainMetadata?.chainId ?? "the expected chain"}. Remove the existing playground network from the wallet, add it again, and retry the payment.`
    );
  }
}

export async function connectWalletToSpawnChain(
  chain: SpawnSession["chain"],
  transport: WalletTransport,
  playgroundMetadata: PlaygroundMetadata | null = null,
  env: Record<string, string | undefined>
) {
  const chainMetadata = resolveSpawnChainMetadata(chain, playgroundMetadata, env);

  if (chainMetadata === null) {
    return;
  }

  try {
    await requestWalletChainSwitch(chainMetadata.chainId, transport);
  } catch (error) {
    const isUnknownChain = isProviderErrorWithCode(error, UNKNOWN_CHAIN_ERROR_CODE);

    if (!isUnknownChain) {
      if (isProviderErrorWithCode(error, USER_REJECTED_REQUEST_ERROR_CODE)) {
        throw new SpawnPaymentError(
          "chain_switch_rejected",
          `Wallet rejected switching to ${chainMetadata.chainName}.`
        );
      }

      throw error;
    }

    if (chainMetadata.rpcUrl === null) {
      throw new SpawnPaymentError(
        "chain_add_failed",
        `Wallet is missing chain ${chainMetadata.chainId} and no RPC URL is configured to add it.`
      );
    }

    try {
      await transport.request({
        method: "wallet_addEthereumChain",
        params: [
          {
            chainId: toHexChainId(chainMetadata.chainId),
            chainName: chainMetadata.chainName,
            rpcUrls: [chainMetadata.rpcUrl],
            nativeCurrency: {
              name: chainMetadata.currencyName,
              symbol: chainMetadata.currencySymbol,
              decimals: 18
            },
            blockExplorerUrls:
              chainMetadata.blockExplorerUrl === null ? [] : [chainMetadata.blockExplorerUrl]
          }
        ]
      });
    } catch (addChainError) {
      if (isProviderErrorWithCode(addChainError, USER_REJECTED_REQUEST_ERROR_CODE)) {
        throw new SpawnPaymentError(
          "chain_add_rejected",
          `Wallet rejected adding ${chainMetadata.chainName}.`
        );
      }

      throw new SpawnPaymentError(
        "chain_add_failed",
        `Wallet could not add ${chainMetadata.chainName}.`
      );
    }

    try {
      await requestWalletChainSwitch(chainMetadata.chainId, transport);
    } catch (switchChainError) {
      if (isProviderErrorWithCode(switchChainError, USER_REJECTED_REQUEST_ERROR_CODE)) {
        throw new SpawnPaymentError(
          "chain_switch_rejected",
          `Wallet rejected switching to ${chainMetadata.chainName}.`
        );
      }

      throw switchChainError;
    }
  }
}

function createAvailability(
  canSubmit: boolean,
  disabledReason: string | null,
  expectedChainId: number | null
): SpawnPaymentAvailability {
  return {
    canSubmit,
    disabledReason,
    expectedChainId
  };
}

export function getSpawnPaymentAvailability(
  session: SpawnSession | null,
  payment: SpawnPaymentInstructions | null,
  wallet: SpawnPaymentWalletState,
  playgroundMetadata: PlaygroundMetadata | null = null,
  env: Record<string, string | undefined> = import.meta.env
): SpawnPaymentAvailability {
  if (session === null || payment === null) {
    return createAvailability(false, "Payment instructions are not available yet.", null);
  }

  const expectedChainId = resolveSpawnChainId(session.chain, playgroundMetadata, env);

  if (session.state !== "awaiting_payment") {
    return createAvailability(
      false,
      "Wallet payment is only available while the session is awaiting payment.",
      expectedChainId
    );
  }

  if (session.paymentStatus === "partial") {
    return createAvailability(
      false,
      "Partial payments require manual recovery. The wizard only submits the original quoted amount once.",
      expectedChainId
    );
  }

  if (session.paymentStatus === "paid" || session.paymentStatus === "refunded") {
    return createAvailability(
      false,
      "Payment was already settled for this session.",
      expectedChainId
    );
  }

  if (wallet.address === null) {
    return createAvailability(false, "Connect a wallet to pay for this spawn.", expectedChainId);
  }

  if (expectedChainId === null) {
    return createAvailability(false, `Unsupported payment chain: ${session.chain}.`, null);
  }

  if (wallet.chainId !== expectedChainId) {
    return createAvailability(
      false,
      `Switch the connected wallet to chain ${expectedChainId} before paying.`,
      expectedChainId
    );
  }

  if (payment.asset === "usdc" && resolveSpawnUsdcContractAddress(session.chain, env) === null) {
    return createAvailability(
      false,
      `USDC contract address is not configured for ${session.chain}.`,
      expectedChainId
    );
  }

  return createAvailability(true, null, expectedChainId);
}

export function formatSpawnPaymentError(error: unknown): string {
  if (error instanceof SpawnPaymentError) {
    return error.message;
  }

  const providerMessage = extractProviderErrorMessage(error);

  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === USER_REJECTED_REQUEST_ERROR_CODE
  ) {
    return "Wallet rejected the payment transaction.";
  }

  const normalizedMessage =
    error instanceof Error
      ? error.message
      : providerMessage;

  if (normalizedMessage !== undefined && normalizedMessage !== null) {
    const normalized = normalizedMessage.toLowerCase();

    if (
      normalized.includes("insufficient funds") &&
      normalized.includes("gas")
    ) {
      return "Connected wallet does not have enough ETH to cover playground gas.";
    }

    if (
      normalized.includes("transfer amount exceeds balance") ||
      normalized.includes("insufficient balance") ||
      normalized.includes("erc20")
    ) {
      return "Connected wallet does not have enough USDC for the quoted deposit.";
    }

    if (
      normalized.includes("expired") ||
      normalized.includes("quote ttl")
    ) {
      return new SpawnPaymentError(
        "quote_expired",
        "This spawn session expired before payment completed. Create a new session if the quote TTL elapsed or the playground reset."
      ).message;
    }
  }

  if (error instanceof Error && error.message.trim() !== "") {
    return error.message;
  }

  if (providerMessage !== null) {
    return providerMessage;
  }

  return "Spawn payment failed.";
}

export async function executeSpawnPayment(
  payment: SpawnPaymentInstructions,
  walletAddress: string,
  transport: WalletTransport,
  playgroundMetadata: PlaygroundMetadata | null = null,
  env: Record<string, string | undefined> = import.meta.env
): Promise<SpawnPaymentExecutionResult> {
  await connectWalletToSpawnChain(payment.chain, transport, playgroundMetadata, env);

  switch (payment.asset) {
    case "usdc": {
      const tokenAddress = resolveSpawnUsdcContractAddress(payment.chain, env);
      if (tokenAddress === null) {
        throw new Error(`USDC contract address is not configured for ${payment.chain}.`);
      }

      const amount = parseRawTokenAmount(payment.grossAmount);
      if (amount === null) {
        throw new Error(`Invalid USDC payment amount: ${payment.grossAmount}`);
      }

      const approveData = encodeErc20ApproveData(payment.paymentAddress, amount);
      if (approveData === null) {
        throw new Error("Unable to encode the USDC approval transaction.");
      }

      const depositData = encodeEscrowDepositData(payment.claimId, amount);
      if (depositData === null) {
        throw new Error("Unable to encode the escrow deposit transaction.");
      }

      await assertWalletTargetsActivePlaygroundNetwork(
        payment,
        transport,
        tokenAddress,
        playgroundMetadata,
        env
      );

      const approvalTxHash = await transport.request<string>({
        method: "eth_sendTransaction",
        params: [
          {
            from: walletAddress,
            to: tokenAddress,
            data: approveData,
            value: bigintToHex(0n)
          }
        ]
      });

      const paymentTxHash = await transport.request<string>({
        method: "eth_sendTransaction",
        params: [
          {
            from: walletAddress,
            to: payment.paymentAddress,
            data: depositData,
            value: bigintToHex(0n)
          }
        ]
      });

      const pendingReceipts: Array<"approval" | "deposit"> = [];

      const [approvalReceiptState, paymentReceiptState] = await Promise.all([
        waitForTransactionReceipt(
          approvalTxHash,
          transport,
          payment.chain,
          "Approval",
          playgroundMetadata,
          env
        ),
        waitForTransactionReceipt(
          paymentTxHash,
          transport,
          payment.chain,
          "Deposit",
          playgroundMetadata,
          env
        )
      ]);
      if (approvalReceiptState === "pending") {
        pendingReceipts.push("approval");
      }
      if (paymentReceiptState === "pending") {
        pendingReceipts.push("deposit");
      }

      return {
        approvalTxHash,
        paymentTxHash,
        pendingReceipts
      };
    }
    default:
      throw new Error(`Unsupported spawn payment asset: ${payment.asset}`);
  }
}
