import { useEffect, useState } from "react";
import type {
  CreateSpawnSessionRequest,
  PlaygroundMetadata,
  SpawnSession
} from "@ic-automaton/shared";

import {
  claimPlaygroundFaucet,
  type PlaygroundFaucetClaimResponse
} from "../../api/playground";
import { fetchOpenRouterModels } from "../../api/openrouter";
import { formatPlaygroundTimestamp } from "../../hooks/usePlayground";
import {
  describeSpawnSessionProgress,
  formatSpawnSessionStateLabel,
  useSpawnSession
} from "../../hooks/useSpawnSession";
import {
  defaultModelOptions,
  type ProviderModelOption
} from "../../lib/default-models";
import {
  connectWalletToSpawnChain,
  executeSpawnPayment,
  formatSpawnPaymentError,
  getSpawnPaymentAvailability,
  type SpawnPaymentExecutionResult
} from "../../lib/spawn-payment";
import {
  encodeErc20BalanceOfData,
  hexQuantityToBigInt,
  parseDecimalAmount,
  resolveSpawnChainId,
  resolveSpawnChainMetadata,
  resolveSpawnUsdcContractAddress
} from "../../lib/wallet-transaction-helpers";
import {
  buildProviderSummary,
  chainOptions,
  createInitialSpawnWizardState,
  describeFundingValidation,
  getActiveChainLabel,
  getFundingPreview,
  getSelectedModel,
  getRiskProfile,
  TOTAL_SPAWN_STEPS,
  type SpawnWizardState
} from "./spawn-state";
import type { WalletSession } from "../../wallet/useWalletSession";
import { ChainStep } from "./steps/ChainStep";
import { FundStep } from "./steps/FundStep";
import { ProviderConfigStep } from "./steps/ProviderConfigStep";
import { RiskStep } from "./steps/RiskStep";

interface SpawnWizardProps {
  isOpen: boolean;
  onClose: () => void;
  onSpawned?: (canisterId: string) => void;
  playgroundError: string | null;
  playgroundIsFallback: boolean;
  playgroundMetadata: PlaygroundMetadata | null;
  walletSession: WalletSession;
}

interface WalletBalanceState {
  error: string | null;
  ethWei: bigint | null;
  isLoading: boolean;
  usdcRaw: bigint | null;
}

type SpawnJourneyStatus = "pending" | "current" | "complete" | "error";

interface SpawnJourneyStep {
  detail: string;
  key: "session" | "payment" | "detection" | "provision" | "complete";
  label: string;
  status: SpawnJourneyStatus;
}

interface SpawnJourneyProgress {
  completedCount: number;
  currentDetail: string;
  currentLabel: string;
  progressPercent: number;
  steps: SpawnJourneyStep[];
  totalCount: number;
}

const stepTitles = [
  "Select chain",
  "Risk appetite",
  "Provider config",
  "Fund"
] as const;
const USDC_DECIMALS = 6;
const MINIMUM_GAS_WEI =
  parseDecimalAmount("0.005", 18) ?? 5_000_000_000_000_000n;

function createEmptyWalletBalanceState(): WalletBalanceState {
  return {
    error: null,
    ethWei: null,
    isLoading: false,
    usdcRaw: null
  };
}

function formatShortHash(value: string): string {
  return `${value.slice(0, 10)}...${value.slice(-8)}`;
}

function describePendingReceipts(
  pendingReceipts: SpawnPaymentExecutionResult["pendingReceipts"]
): string {
  if (pendingReceipts.length === 0) {
    return "";
  }

  if (pendingReceipts.length === 1) {
    return pendingReceipts[0] === "approval"
      ? "approval transaction"
      : "deposit transaction";
  }

  return "approval and deposit transactions";
}

export function getEffectivePendingReceipts(
  session: Pick<SpawnSession, "state" | "paymentStatus"> | null,
  paymentResult: SpawnPaymentExecutionResult | null
): SpawnPaymentExecutionResult["pendingReceipts"] {
  if (paymentResult === null) {
    return [];
  }

  if (session === null) {
    return [...paymentResult.pendingReceipts];
  }

  if (session.paymentStatus === "paid" || session.state !== "awaiting_payment") {
    return [];
  }

  if (session.paymentStatus === "partial") {
    return ["deposit"];
  }

  return [...paymentResult.pendingReceipts];
}

export function getPaymentSubmissionLockReason(
  session: Pick<SpawnSession, "state" | "paymentStatus"> | null,
  paymentResult: SpawnPaymentExecutionResult | null,
  pendingReceipts: SpawnPaymentExecutionResult["pendingReceipts"]
): string | null {
  if (paymentResult === null || session === null) {
    return null;
  }

  if (session.state !== "awaiting_payment") {
    return null;
  }

  if (session.paymentStatus !== "unpaid") {
    return null;
  }

  return pendingReceipts.length > 0
    ? "Wallet payment was already submitted for this session. Confirm or cancel the queued wallet transactions instead of sending another pair."
    : "Wallet payment was already submitted for this session. Wait for the factory/indexer to mirror it instead of sending another pair.";
}

function formatTokenAmount(
  value: bigint | null,
  decimals: number,
  symbol: string,
  precision = 4
): string {
  if (value === null) {
    return "Pending";
  }

  const divisor = 10n ** BigInt(decimals);
  const whole = value / divisor;
  const fraction = value % divisor;

  if (decimals === 0) {
    return `${whole.toString()} ${symbol}`;
  }

  const paddedFraction = fraction
    .toString()
    .padStart(decimals, "0")
    .replace(/0+$/, "")
    .slice(0, precision);

  return paddedFraction === ""
    ? `${whole.toString()} ${symbol}`
    : `${whole.toString()}.${paddedFraction} ${symbol}`;
}

function formatClaimWindow(seconds: number): string {
  if (seconds % 86_400 === 0) {
    return `${seconds / 86_400}d`;
  }

  if (seconds % 3_600 === 0) {
    return `${seconds / 3_600}h`;
  }

  if (seconds % 60 === 0) {
    return `${seconds / 60}m`;
  }

  return `${seconds}s`;
}

function createSpawnJourneySteps(): SpawnJourneyStep[] {
  return [
    {
      key: "session",
      label: "Create session",
      detail: "Factory prepares the quoted escrow session and payment instructions.",
      status: "pending"
    },
    {
      key: "payment",
      label: "Confirm wallet payment",
      detail: "Approve USDC and submit the escrow deposit from the connected wallet.",
      status: "pending"
    },
    {
      key: "detection",
      label: "Detect funding",
      detail: "The factory and indexer mark the session paid after the deposit lands.",
      status: "pending"
    },
    {
      key: "provision",
      label: "Provision automaton",
      detail: "The factory creates the automaton canister and applies its initial configuration.",
      status: "pending"
    },
    {
      key: "complete",
      label: "Complete",
      detail: "The new automaton appears on the grid and the session closes.",
      status: "pending"
    }
  ];
}

function setSpawnJourneyStep(
  steps: SpawnJourneyStep[],
  key: SpawnJourneyStep["key"],
  status: SpawnJourneyStatus,
  detail?: string
) {
  const step = steps.find((candidate) => candidate.key === key);
  if (step === undefined) {
    return;
  }

  step.status = status;

  if (detail !== undefined) {
    step.detail = detail;
  }
}

function finalizeSpawnJourneyProgress(
  steps: SpawnJourneyStep[],
  currentLabel: string,
  currentDetail: string
): SpawnJourneyProgress {
  const completedCount = steps.filter((step) => step.status === "complete").length;
  const totalCount = steps.length;
  const progressPercent =
    completedCount === totalCount
      ? 100
      : ((completedCount + 0.5) / totalCount) * 100;

  return {
    completedCount,
    currentDetail,
    currentLabel,
    progressPercent,
    steps,
    totalCount
  };
}

export function formatSpawnJourneyStatusLabel(status: SpawnJourneyStatus): string {
  switch (status) {
    case "complete":
      return "Done";
    case "current":
      return "In progress";
    case "error":
      return "Attention";
    default:
      return "Waiting";
  }
}

export function deriveSpawnJourneyProgress(
  session: Pick<
    SpawnSession,
    "state" | "paymentStatus" | "refundable" | "retryable"
  > | null,
  paymentResult: SpawnPaymentExecutionResult | null,
  pendingReceipts: SpawnPaymentExecutionResult["pendingReceipts"],
  isCreating: boolean,
  isSubmittingPayment: boolean
): SpawnJourneyProgress | null {
  if (session === null && !isCreating) {
    return null;
  }

  const steps = createSpawnJourneySteps();

  if (session === null) {
    setSpawnJourneyStep(
      steps,
      "session",
      "current",
      "Generating the session ID, quote, and escrow destination."
    );

    return finalizeSpawnJourneyProgress(
      steps,
      "Creating factory session",
      "Factory is preparing the live payment session for this spawn attempt."
    );
  }

  setSpawnJourneyStep(
    steps,
    "session",
    "complete",
    "Session is ready with a locked quote and escrow destination."
  );

  switch (session.state) {
    case "awaiting_payment":
      if (isSubmittingPayment) {
        setSpawnJourneyStep(
          steps,
          "payment",
          "current",
          "Check the wallet and confirm both the USDC approval and escrow deposit."
        );

        return finalizeSpawnJourneyProgress(
          steps,
          "Waiting for wallet confirmation",
          "The wallet is preparing the approval and deposit transactions for this session."
        );
      }

      if (paymentResult !== null && pendingReceipts.length > 0) {
        setSpawnJourneyStep(
          steps,
          "payment",
          "current",
          `Transactions were submitted. Waiting for the ${describePendingReceipts(
            pendingReceipts
          )} to confirm on-chain.`
        );

        return finalizeSpawnJourneyProgress(
          steps,
          "Confirming on-chain payment",
          "Keep the session open while the playground confirms the submitted wallet transactions."
        );
      }

      if (paymentResult !== null || session.paymentStatus === "partial") {
        setSpawnJourneyStep(
          steps,
          "payment",
          "complete",
          "Wallet approval and deposit were submitted from the connected wallet."
        );
        setSpawnJourneyStep(
          steps,
          "detection",
          "current",
          session.paymentStatus === "partial"
            ? "The payment is only partially mirrored so far. Waiting for the full quoted amount to settle."
            : "On-chain payment is confirmed. Waiting for the factory and indexer to mirror it."
        );

        return finalizeSpawnJourneyProgress(
          steps,
          "Waiting for factory confirmation",
          session.paymentStatus === "partial"
            ? "The escrow session shows partial funding. The factory will continue once the full quote is recognized."
            : "The wallet payment landed, but the session is still waiting for the backend to mark it as paid."
        );
      }

      setSpawnJourneyStep(
        steps,
        "payment",
        "current",
        "Use Pay with wallet to approve USDC and submit the escrow deposit for this session."
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Waiting for wallet payment",
        "The factory session is ready. The next step is to authorize the wallet payment."
      );

    case "payment_detected":
      setSpawnJourneyStep(
        steps,
        "payment",
        "complete",
        "Wallet payment reached the escrow session."
      );
      setSpawnJourneyStep(
        steps,
        "detection",
        "complete",
        "Factory detected the quoted escrow payment."
      );
      setSpawnJourneyStep(
        steps,
        "provision",
        "current",
        "Funding is confirmed. The factory is preparing the spawn pipeline."
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Provisioning automaton",
        "Escrow payment is locked in and the factory has started the spawn pipeline."
      );

    case "spawning":
      setSpawnJourneyStep(
        steps,
        "payment",
        "complete",
        "Wallet payment reached the escrow session."
      );
      setSpawnJourneyStep(
        steps,
        "detection",
        "complete",
        "Factory detected the quoted escrow payment."
      );
      setSpawnJourneyStep(
        steps,
        "provision",
        "current",
        "The factory is creating the automaton canister and applying its initial configuration."
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Provisioning automaton",
        "The automaton is being created and configured now."
      );

    case "broadcasting_release":
      setSpawnJourneyStep(
        steps,
        "payment",
        "complete",
        "Wallet payment reached the escrow session."
      );
      setSpawnJourneyStep(
        steps,
        "detection",
        "complete",
        "Factory detected the quoted escrow payment."
      );
      setSpawnJourneyStep(
        steps,
        "provision",
        "current",
        "Provisioning succeeded. Finalizing the release transaction and registry update."
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Finalizing release",
        "The automaton is provisioned. The session is finishing its release and registry work."
      );

    case "complete":
      setSpawnJourneyStep(
        steps,
        "payment",
        "complete",
        "Wallet payment reached the escrow session."
      );
      setSpawnJourneyStep(
        steps,
        "detection",
        "complete",
        "Factory detected the quoted escrow payment."
      );
      setSpawnJourneyStep(
        steps,
        "provision",
        "complete",
        "Factory created the automaton and finalized the release."
      );
      setSpawnJourneyStep(
        steps,
        "complete",
        "complete",
        "Spawn completed and the new automaton should now appear on the grid."
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Spawn complete",
        "The spawn finished successfully. The new automaton should now be visible on the grid."
      );

    case "failed":
      setSpawnJourneyStep(
        steps,
        "payment",
        session.paymentStatus === "unpaid" ? "error" : "complete",
        session.paymentStatus === "unpaid"
          ? "The session failed before a valid wallet payment was completed."
          : "Wallet payment reached the escrow session."
      );

      if (session.paymentStatus === "paid") {
        setSpawnJourneyStep(
          steps,
          "detection",
          "complete",
          "Factory detected the quoted escrow payment."
        );
      } else if (session.paymentStatus !== "unpaid") {
        setSpawnJourneyStep(
          steps,
          "detection",
          "error",
          "Funding was not mirrored cleanly before the session failed."
        );
      }

      setSpawnJourneyStep(
        steps,
        "provision",
        "error",
        describeSpawnSessionProgress(session as SpawnSession)
      );

      return finalizeSpawnJourneyProgress(
        steps,
        "Spawn failed",
        describeSpawnSessionProgress(session as SpawnSession)
      );

    case "expired":
      if (session.paymentStatus === "unpaid") {
        setSpawnJourneyStep(
          steps,
          "payment",
          "error",
          "The quote expired before the wallet payment completed."
        );
      } else {
        setSpawnJourneyStep(
          steps,
          "payment",
          "complete",
          session.paymentStatus === "refunded"
            ? "A wallet payment was submitted and later refunded."
            : "Wallet payment reached the escrow session."
        );
      }

      if (session.paymentStatus === "paid" || session.paymentStatus === "refunded") {
        setSpawnJourneyStep(
          steps,
          "detection",
          "complete",
          "Factory detected the quoted escrow payment."
        );
        setSpawnJourneyStep(
          steps,
          "provision",
          "error",
          "The session expired before the spawn could finish."
        );
      } else if (session.paymentStatus === "partial") {
        setSpawnJourneyStep(
          steps,
          "detection",
          "error",
          "Only part of the quoted payment was mirrored before the session expired."
        );
      }

      return finalizeSpawnJourneyProgress(
        steps,
        "Session expired",
        describeSpawnSessionProgress(session as SpawnSession)
      );

    default:
      return finalizeSpawnJourneyProgress(
        steps,
        "Session in progress",
        describeSpawnSessionProgress(session as SpawnSession)
      );
  }
}

async function requestRpcHexValue(
  rpcUrl: string,
  method: string,
  params: unknown[]
): Promise<string> {
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

  if (typeof payload.result !== "string") {
    throw new Error(`${method} returned an unreadable response.`);
  }

  return payload.result;
}

function formatFaucetAmounts(metadata: PlaygroundMetadata | null): string {
  if (metadata === null) {
    return "Faucet amounts pending.";
  }

  return metadata.faucet.claimAssetAmounts
    .map((entry) => `${entry.amount} ${entry.asset.toUpperCase()}`)
    .join(" + ");
}

function buildExplorerTransactionUrl(
  explorerUrl: string | null,
  txHash: string
): string | null {
  if (explorerUrl === null) {
    return null;
  }

  try {
    return new URL(`tx/${txHash}`, `${explorerUrl.replace(/\/?$/, "/")}`).toString();
  } catch {
    return null;
  }
}

export function SpawnWizard({
  isOpen,
  onClose,
  onSpawned,
  playgroundError,
  playgroundIsFallback,
  playgroundMetadata,
  walletSession
}: SpawnWizardProps) {
  const [stepIndex, setStepIndex] = useState(0);
  const [state, setState] = useState<SpawnWizardState>(
    createInitialSpawnWizardState()
  );
  const [reportedCompletionSessionId, setReportedCompletionSessionId] = useState<
    string | null
  >(null);
  const [modelOptions, setModelOptions] =
    useState<ProviderModelOption[]>(defaultModelOptions);
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [modelStatusMessage, setModelStatusMessage] = useState(
    "Using curated fallback models until the live catalog is requested."
  );
  const [isSubmittingPayment, setIsSubmittingPayment] = useState(false);
  const [paymentError, setPaymentError] = useState<string | null>(null);
  const [paymentResult, setPaymentResult] =
    useState<SpawnPaymentExecutionResult | null>(null);
  const [isSwitchingNetwork, setIsSwitchingNetwork] = useState(false);
  const [networkActionError, setNetworkActionError] = useState<string | null>(
    null
  );
  const [networkActionMessage, setNetworkActionMessage] = useState<string | null>(
    null
  );
  const [isClaimingFaucet, setIsClaimingFaucet] = useState(false);
  const [faucetError, setFaucetError] = useState<string | null>(null);
  const [faucetResult, setFaucetResult] =
    useState<PlaygroundFaucetClaimResponse | null>(null);
  const [walletBalances, setWalletBalances] = useState<WalletBalanceState>(
    createEmptyWalletBalanceState()
  );
  const [balanceRefreshToken, setBalanceRefreshToken] = useState(0);
  const spawnSession = useSpawnSession();
  const viewerAddress = walletSession.address;

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    const controller = new AbortController();

    setIsLoadingModels(true);
    setModelStatusMessage("Loading live OpenRouter models.");

    void fetchOpenRouterModels(controller.signal)
      .then((models) => {
        setModelOptions(models);
        setModelStatusMessage("Loaded live OpenRouter models.");
      })
      .catch(() => {
        setModelOptions(defaultModelOptions);
        setModelStatusMessage(
          "OpenRouter catalog unavailable, using curated fallback models."
        );
      })
      .finally(() => {
        setIsLoadingModels(false);
      });

    return () => {
      controller.abort();
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };

    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, onClose]);

  const fundingPreview = getFundingPreview(state);
  const validationMessage = describeFundingValidation(fundingPreview);
  const hasTrackedSession = spawnSession.sessionId !== null;
  const activeSession = spawnSession.session;
  const paymentInstructions = spawnSession.paymentInstructions;
  const effectivePendingReceipts = getEffectivePendingReceipts(
    activeSession,
    paymentResult
  );
  const expectedChainId = resolveSpawnChainId(state.chain, playgroundMetadata);
  const walletOnExpectedChain =
    expectedChainId !== null && walletSession.chainId === expectedChainId;
  const requiredUsdcRaw =
    state.asset === "usdc"
      ? parseDecimalAmount(state.grossAmountInput, USDC_DECIMALS)
      : null;
  const hasEnoughEth =
    walletBalances.ethWei === null
      ? null
      : walletBalances.ethWei >= MINIMUM_GAS_WEI;
  const hasEnoughUsdc =
    walletBalances.usdcRaw === null || requiredUsdcRaw === null
      ? null
      : walletBalances.usdcRaw >= requiredUsdcRaw;
  const hasKnownFundingShortfall =
    hasEnoughEth === false || hasEnoughUsdc === false;
  const canSubmit =
    viewerAddress !== null &&
    walletOnExpectedChain &&
    state.chain === "base" &&
    fundingPreview.minimumMet &&
    fundingPreview.grossAmount > 0 &&
    !spawnSession.isCreating &&
    !playgroundMetadata?.maintenance &&
    !hasKnownFundingShortfall;

  useEffect(() => {
    if (
      activeSession?.state !== "complete" ||
      activeSession.automatonCanisterId === null ||
      activeSession.sessionId === reportedCompletionSessionId
    ) {
      return;
    }

    onSpawned?.(activeSession.automatonCanisterId);
    setReportedCompletionSessionId(activeSession.sessionId);
  }, [activeSession, onSpawned, reportedCompletionSessionId]);

  useEffect(() => {
    setPaymentError(null);
    setPaymentResult(null);
    setIsSubmittingPayment(false);
  }, [spawnSession.sessionId]);

  useEffect(() => {
    setNetworkActionError(null);
    setNetworkActionMessage(null);
  }, [walletSession.chainId, walletSession.selectedProviderId]);

  useEffect(() => {
    if (!isOpen || viewerAddress === null || !walletOnExpectedChain) {
      setWalletBalances(createEmptyWalletBalanceState());
      return;
    }

    const chainMetadata = resolveSpawnChainMetadata(
      state.chain,
      playgroundMetadata
    );
    const rpcUrl = chainMetadata?.rpcUrl;
    const usdcContractAddress = resolveSpawnUsdcContractAddress(
      state.chain,
      import.meta.env
    );
    const balanceOfData = encodeErc20BalanceOfData(viewerAddress);

    if (rpcUrl === null || rpcUrl === undefined) {
      setWalletBalances({
        error: "Playground RPC URL is unavailable for balance checks.",
        ethWei: null,
        isLoading: false,
        usdcRaw: null
      });
      return;
    }

    if (balanceOfData === null) {
      setWalletBalances({
        error: "Unable to encode the playground USDC balance request.",
        ethWei: null,
        isLoading: false,
        usdcRaw: null
      });
      return;
    }

    let cancelled = false;

    setWalletBalances((current) => ({
      ...current,
      error: null,
      isLoading: true
    }));

    void Promise.all([
      requestRpcHexValue(rpcUrl, "eth_getBalance", [viewerAddress, "latest"]),
      usdcContractAddress === null
        ? Promise.resolve<string | null>(null)
        : requestRpcHexValue(rpcUrl, "eth_call", [
            {
              data: balanceOfData,
              to: usdcContractAddress
            },
            "latest"
          ])
    ])
      .then(([ethBalanceHex, usdcBalanceHex]) => {
        if (cancelled) {
          return;
        }

        const ethWei = hexQuantityToBigInt(ethBalanceHex);
        const usdcRaw =
          usdcBalanceHex === null ? null : hexQuantityToBigInt(usdcBalanceHex);

        if (ethWei === null || (usdcBalanceHex !== null && usdcRaw === null)) {
          setWalletBalances({
            error: "Wallet returned an unreadable playground balance payload.",
            ethWei: null,
            isLoading: false,
            usdcRaw: null
          });
          return;
        }

        setWalletBalances({
          error: null,
          ethWei,
          isLoading: false,
          usdcRaw
        });
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }

        setWalletBalances({
          error: formatSpawnPaymentError(error),
          ethWei: null,
          isLoading: false,
          usdcRaw: null
        });
      });

    return () => {
      cancelled = true;
    };
  }, [
    balanceRefreshToken,
    isOpen,
    playgroundMetadata,
    state.chain,
    viewerAddress,
    walletOnExpectedChain,
    walletSession.selectedProviderId
  ]);

  function resetWizard() {
    setStepIndex(0);
    setState(createInitialSpawnWizardState());
    setModelOptions(defaultModelOptions);
    setModelStatusMessage(
      "Using curated fallback models until the live catalog is requested."
    );
    setIsLoadingModels(false);
    setIsSubmittingPayment(false);
    setPaymentError(null);
    setPaymentResult(null);
    setIsSwitchingNetwork(false);
    setNetworkActionError(null);
    setNetworkActionMessage(null);
    setIsClaimingFaucet(false);
    setFaucetError(null);
    setFaucetResult(null);
    setWalletBalances(createEmptyWalletBalanceState());
    setBalanceRefreshToken(0);
  }

  function closeWizard() {
    if (!hasTrackedSession) {
      resetWizard();
    }

    onClose();
  }

  function advanceStep() {
    setStepIndex((current) =>
      current < TOTAL_SPAWN_STEPS - 1 ? current + 1 : current
    );
  }

  function retreatStep() {
    setStepIndex((current) => (current > 0 ? current - 1 : current));
  }

  function resetTrackedSession() {
    spawnSession.reset();
    resetWizard();
  }

  function buildCreateRequest(): CreateSpawnSessionRequest | null {
    if (viewerAddress === null) {
      return null;
    }

    const grossAmount =
      state.asset === "usdc"
        ? parseDecimalAmount(state.grossAmountInput, USDC_DECIMALS)?.toString() ?? null
        : null;

    if (grossAmount === null) {
      return null;
    }

    return {
      stewardAddress: viewerAddress,
      asset: state.asset,
      grossAmount,
      config: {
        chain: state.chain,
        risk: state.risk,
        strategies: [...state.strategies],
        skills: [...state.skills],
        provider: {
          openRouterApiKey:
            state.openRouterApiKey.trim() === ""
              ? null
              : state.openRouterApiKey.trim(),
          model: getSelectedModel(state),
          braveSearchApiKey:
            state.braveSearchApiKey.trim() === ""
              ? null
              : state.braveSearchApiKey.trim()
        }
      }
    };
  }

  async function submitPaymentForSession(
    payment:
      | {
          sessionId: string;
          claimId: string;
          chain: "base";
          asset: "usdc";
          paymentAddress: string;
          grossAmount: string;
          quoteTermsHash: string;
          expiresAt: number;
        }
      | null
  ) {
    if (viewerAddress === null || payment === null) {
      return;
    }

    setIsSubmittingPayment(true);
    setPaymentError(null);
    setPaymentResult(null);

    try {
      const result = await executeSpawnPayment(
        payment,
        viewerAddress,
        walletSession,
        playgroundMetadata
      );
      setPaymentResult(result);
      setBalanceRefreshToken((current) => current + 1);
    } catch (error) {
      setPaymentError(formatSpawnPaymentError(error));
    } finally {
      setIsSubmittingPayment(false);
    }
  }

  async function handleSubmit() {
    if (!canSubmit || hasTrackedSession) {
      return;
    }

    const request = buildCreateRequest();

    if (request === null) {
      return;
    }

    const response = await spawnSession.create(request);

    if (response === null) {
      return;
    }

    await submitPaymentForSession(response.quote.payment);
  }

  const paymentAvailability = getSpawnPaymentAvailability(
    activeSession,
    paymentInstructions,
    walletSession,
    playgroundMetadata
  );
  const paymentSubmissionLockReason = getPaymentSubmissionLockReason(
    activeSession,
    paymentResult,
    effectivePendingReceipts
  );
  const canSubmitPaymentAction =
    paymentAvailability.canSubmit &&
    paymentSubmissionLockReason === null &&
    !isSubmittingPayment;
  const paymentActionDisabledReason =
    paymentSubmissionLockReason ?? paymentAvailability.disabledReason;
  const showPaymentAction =
    paymentInstructions !== null &&
    (paymentAvailability.canSubmit || isSubmittingPayment);
  const showRetryAction = activeSession?.retryable ?? false;
  const showRefundAction = activeSession?.refundable ?? false;
  const showSessionActions =
    showPaymentAction || showRetryAction || showRefundAction;
  const spawnJourney = deriveSpawnJourneyProgress(
    activeSession,
    paymentResult,
    effectivePendingReceipts,
    spawnSession.isCreating,
    isSubmittingPayment
  );
  const headerMetaLabel =
    spawnJourney === null
      ? `Step ${stepIndex + 1} of ${TOTAL_SPAWN_STEPS}`
      : `Live progress · ${spawnJourney.completedCount} / ${spawnJourney.totalCount} complete`;
  const headerMetaTitle = spawnJourney?.currentLabel ?? stepTitles[stepIndex];
  const progressPercent =
    spawnJourney?.progressPercent ??
    ((stepIndex + 1) / TOTAL_SPAWN_STEPS) * 100;

  async function handlePayment() {
    if (
      activeSession === null ||
      paymentInstructions === null ||
      viewerAddress === null ||
      !canSubmitPaymentAction ||
      isSubmittingPayment
    ) {
      return;
    }

    await submitPaymentForSession(paymentInstructions);
  }

  async function handleWalletConnect() {
    await walletSession.connect();
  }

  async function handleNetworkAction() {
    if (!walletSession.hasProvider) {
      setNetworkActionError("No injected wallet provider is available.");
      return;
    }

    setIsSwitchingNetwork(true);
    setNetworkActionError(null);
    setNetworkActionMessage(null);

    try {
      await connectWalletToSpawnChain(
        state.chain,
        walletSession,
        playgroundMetadata,
        import.meta.env
      );
      setNetworkActionMessage(
        `Wallet is ready on ${playgroundMetadata?.chain.name ?? "the playground network"}.`
      );
      setBalanceRefreshToken((current) => current + 1);
    } catch (error) {
      setNetworkActionError(formatSpawnPaymentError(error));
    } finally {
      setIsSwitchingNetwork(false);
    }
  }

  async function handleClaimFaucet() {
    if (viewerAddress === null) {
      setFaucetError("Connect a wallet before claiming playground funds.");
      return;
    }

    if (playgroundMetadata === null || !playgroundMetadata.faucet.available) {
      setFaucetError("Playground faucet is currently unavailable.");
      return;
    }

    if (playgroundMetadata.maintenance) {
      setFaucetError(
        "Playground is in maintenance mode while the reset completes."
      );
      return;
    }

    setIsClaimingFaucet(true);
    setFaucetError(null);
    setFaucetResult(null);

    try {
      const result = await claimPlaygroundFaucet(viewerAddress);
      setFaucetResult(result);
      setWalletBalances({
        error: null,
        ethWei: BigInt(result.balances.ethWei),
        isLoading: false,
        usdcRaw: BigInt(result.balances.usdcRaw)
      });
      setBalanceRefreshToken((current) => current + 1);
    } catch (error) {
      setFaucetError(formatSpawnPaymentError(error));
    } finally {
      setIsClaimingFaucet(false);
    }
  }

  const playgroundChainName =
    playgroundMetadata?.chain.name ?? getActiveChainLabel(state.chain);
  const playgroundNote =
    playgroundMetadata === null
      ? "Canisters, balances, and session state are non-durable in this playground."
      : playgroundMetadata.maintenance
        ? `Maintenance mode is active. Last reset ${formatPlaygroundTimestamp(playgroundMetadata.reset.lastResetAt, "pending")} · next window ${formatPlaygroundTimestamp(playgroundMetadata.reset.nextResetAt, "pending")}. New sessions are paused while the reset completes.`
        : `Last reset ${formatPlaygroundTimestamp(playgroundMetadata.reset.lastResetAt, "pending")} · next window ${formatPlaygroundTimestamp(playgroundMetadata.reset.nextResetAt, "pending")} · ${playgroundMetadata.reset.cadenceLabel}. Canisters, balances, and session state are non-durable.`;
  const connectButtonLabel =
    viewerAddress !== null
      ? "Wallet connected"
      : walletSession.selectedProviderName !== null
        ? `Connect ${walletSession.selectedProviderName}`
        : "Connect wallet";
  const walletStatusMessage = !walletSession.hasProvider
    ? "No injected wallet detected. Install or enable MetaMask, Rabby, or another EIP-6963 wallet."
    : viewerAddress === null
      ? "Choose the wallet you want to fund, then connect it here before spawning."
      : walletSession.selectedProviderName !== null
        ? `${walletSession.selectedProviderName} is connected for playground funding and payment.`
        : "Wallet is connected for playground funding and payment.";
  const networkStatusMessage = networkActionMessage
    ? networkActionMessage
    : !walletSession.hasProvider
      ? "A wallet provider is required before the playground network can be added."
      : walletOnExpectedChain
        ? `Wallet is already on ${playgroundChainName}.`
        : walletSession.chainId === null
          ? `Use the button below to add and switch to ${playgroundChainName}.`
          : `Wallet is connected to chain ${walletSession.chainId}. Switch to ${playgroundChainName} before spawning.`;
  const faucetDisabledReason = playgroundMetadata?.maintenance
    ? "Maintenance is active while the playground reset completes."
    : playgroundMetadata === null
      ? "Playground metadata is unavailable."
      : !playgroundMetadata.faucet.available
        ? "Faucet unavailable."
        : viewerAddress === null
          ? "Connect the wallet you want to fund."
          : null;
  const faucetStatusMessage =
    faucetResult !== null
      ? `Faucet funded ${formatFaucetAmounts(playgroundMetadata)} for the connected wallet.`
      : playgroundMetadata === null
        ? "Faucet limits are unavailable until playground metadata loads."
        : `Faucet sends ${formatFaucetAmounts(playgroundMetadata)}. Limit ${playgroundMetadata.faucet.claimLimits.maxClaimsPerWallet} wallet / ${playgroundMetadata.faucet.claimLimits.maxClaimsPerIp} IP every ${formatClaimWindow(playgroundMetadata.faucet.claimLimits.windowSeconds)}.`;
  const faucetTransactions =
    faucetResult === null
      ? []
      : ([
          {
            asset: "eth" as const,
            hash: faucetResult.txHashes.eth,
            href: buildExplorerTransactionUrl(
              playgroundMetadata?.chain.explorerUrl ?? null,
              faucetResult.txHashes.eth
            )
          },
          {
            asset: "usdc" as const,
            hash: faucetResult.txHashes.usdc,
            href: buildExplorerTransactionUrl(
              playgroundMetadata?.chain.explorerUrl ?? null,
              faucetResult.txHashes.usdc
            )
          }
        ] as const);
  const ethBalance = viewerAddress === null
    ? "Wallet required"
    : !walletOnExpectedChain
      ? "Switch to playground"
      : formatTokenAmount(walletBalances.ethWei, 18, "ETH");
  const usdcBalance = viewerAddress === null
    ? "Wallet required"
    : !walletOnExpectedChain
      ? "Switch to playground"
      : formatTokenAmount(walletBalances.usdcRaw, USDC_DECIMALS, "USDC", 2);
  const ethStatus = viewerAddress === null
    ? "Connect wallet"
    : !walletOnExpectedChain
      ? "Wrong chain"
      : walletBalances.isLoading
        ? "Checking"
        : hasEnoughEth === false
          ? "Insufficient ETH for gas"
          : hasEnoughEth === true
            ? "Ready for gas"
            : "Balance pending";
  const usdcStatus = viewerAddress === null
    ? "Connect wallet"
    : !walletOnExpectedChain
      ? "Wrong chain"
      : walletBalances.isLoading
        ? "Checking"
        : hasEnoughUsdc === false
          ? "Insufficient USDC"
          : hasEnoughUsdc === true
            ? "Ready for payment"
            : "Balance pending";

  return (
    <div
      aria-hidden={!isOpen}
      className={`spawn-overlay${isOpen ? " is-open" : ""}`}
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          closeWizard();
        }
      }}
    >
      <section
        aria-label="Spawn automaton wizard"
        aria-modal="true"
        className="spawn-wizard"
        role="dialog"
      >
        <button
          aria-label="Close spawn wizard"
          className="spawn-close"
          onClick={closeWizard}
          type="button"
        >
          &times;
        </button>

        <header className="spawn-header">
          <div>
            <p className="section-label">Spawn wizard</p>
            <h2 className="spawn-heading">Spawn Automaton</h2>
          </div>
          <div className="spawn-header-meta">
            <span>{headerMetaLabel}</span>
            <strong>{headerMetaTitle}</strong>
          </div>
        </header>

        <div className="spawn-progress">
          <div
            className="spawn-progress-fill"
            style={{
              width: `${progressPercent}%`
            }}
          />
        </div>

        <div className="spawn-body">
          {stepIndex === 0 ? (
            <ChainStep
              onChange={(chain) => {
                setState((current) => ({
                  ...current,
                  chain
                }));
              }}
              value={state.chain}
            />
          ) : null}

          {stepIndex === 1 ? (
            <RiskStep
              onChange={(risk) => {
                setState((current) => ({
                  ...current,
                  risk
                }));
              }}
              value={state.risk}
            />
          ) : null}

          {stepIndex === 2 ? (
            <ProviderConfigStep
              braveSearchApiKey={state.braveSearchApiKey}
              customModelId={state.customModelId}
              isLoadingModels={isLoadingModels}
              modelOptions={modelOptions}
              modelStatusMessage={modelStatusMessage}
              onBraveSearchApiKeyChange={(value) => {
                setState((current) => ({
                  ...current,
                  braveSearchApiKey: value
                }));
              }}
              onCustomModelChange={(value) => {
                setState((current) => ({
                  ...current,
                  customModelId: value
                }));
              }}
              onOpenRouterApiKeyChange={(value) => {
                setState((current) => ({
                  ...current,
                  openRouterApiKey: value
                }));
              }}
              onSelectedModelChange={(value) => {
                setState((current) => ({
                  ...current,
                  selectedModelId: value
                }));
              }}
              openRouterApiKey={state.openRouterApiKey}
              selectedModelId={state.selectedModelId}
            />
          ) : null}

          {stepIndex === 3 ? (
            <FundStep
              asset={state.asset}
              balances={{
                errorMessage: walletBalances.error,
                ethBalance,
                ethStatus,
                isLoading: walletBalances.isLoading,
                usdcBalance,
                usdcStatus
              }}
              faucet={{
                actionLabel: "Get test funds",
                disabledReason: faucetDisabledReason,
                errorMessage: faucetError,
                isPending: isClaimingFaucet,
                statusMessage: faucetStatusMessage,
                txLinks: faucetTransactions.map((transaction) => ({
                  asset: transaction.asset,
                  hash: formatShortHash(transaction.hash),
                  href: transaction.href
                }))
              }}
              grossAmountInput={state.grossAmountInput}
              network={{
                actionLabel: "Add / switch playground network",
                disabled:
                  !walletSession.hasProvider ||
                  isSwitchingNetwork ||
                  walletOnExpectedChain ||
                  expectedChainId === null,
                errorMessage: networkActionError,
                isPending: isSwitchingNetwork,
                statusMessage: networkStatusMessage
              }}
              onAssetChange={(asset) => {
                setState((current) => ({
                  ...current,
                  asset
                }));
              }}
              onClaimFaucet={() => {
                void handleClaimFaucet();
              }}
              onConnectWallet={() => {
                void handleWalletConnect();
              }}
              onGrossAmountChange={(grossAmountInput) => {
                setState((current) => ({
                  ...current,
                  grossAmountInput
                }));
              }}
              onNetworkAction={() => {
                void handleNetworkAction();
              }}
              onProviderChange={(providerId) => {
                walletSession.setSelectedProvider(providerId);
              }}
              playground={{
                chainId: expectedChainId,
                chainName: playgroundChainName,
                environmentLabel:
                  playgroundMetadata?.environmentLabel ?? "Playground metadata pending",
                maintenance: playgroundMetadata?.maintenance ?? false,
                note: playgroundNote,
                runtimeError: playgroundError,
                usesFallback: playgroundIsFallback
              }}
              preview={fundingPreview}
              summary={{
                chain:
                  chainOptions.find((option) => option.id === state.chain)?.label ??
                  getActiveChainLabel(state.chain),
                risk: getRiskProfile(state.risk).label,
                strategies: state.strategies.length,
                skills: state.skills.length,
                providerModel: buildProviderSummary(state),
                braveConfigured: state.braveSearchApiKey.trim() !== ""
              }}
              validationMessage={validationMessage}
              wallet={{
                address: viewerAddress,
                connectLabel: connectButtonLabel,
                errorMessage: walletSession.errorMessage,
                hasProvider: walletSession.hasProvider,
                isConnecting: walletSession.isConnecting,
                providerOptions: walletSession.providers,
                selectedProviderId: walletSession.selectedProviderId,
                statusMessage: walletStatusMessage
              }}
            />
          ) : null}

          {spawnJourney !== null ? (
            <section className="spawn-session-status" aria-live="polite">
              <div className="spawn-session-header">
                <div>
                  <p className="section-label">Live spawn progress</p>
                  <h3 className="spawn-step-title">Funding and Provisioning</h3>
                </div>
                <span className="spawn-session-pill">
                  {spawnJourney.currentLabel}
                </span>
              </div>

              <p className="spawn-step-copy">
                {spawnJourney.currentDetail}
              </p>

              <div className="spawn-journey-overview">
                <div className="spawn-journey-overview-head">
                  <strong>
                    {spawnJourney.completedCount} of {spawnJourney.totalCount} steps
                    complete
                  </strong>
                  <span>{headerMetaTitle}</span>
                </div>
                <ol className="spawn-journey-list">
                  {spawnJourney.steps.map((step) => (
                    <li
                      className={`spawn-journey-step is-${step.status}`}
                      key={step.key}
                    >
                      <span aria-hidden="true" className="spawn-journey-marker" />
                      <div className="spawn-journey-content">
                        <p className="spawn-journey-label">{step.label}</p>
                        <p className="spawn-journey-copy">{step.detail}</p>
                      </div>
                      <span className="spawn-journey-step-status">
                        {formatSpawnJourneyStatusLabel(step.status)}
                      </span>
                    </li>
                  ))}
                </ol>
              </div>

              {activeSession !== null ? (
                <>
                  {showSessionActions ? (
                    <div className="spawn-session-actions">
                      {showPaymentAction ? (
                        <button
                          className="spawn-nav-button is-primary"
                          disabled={!canSubmitPaymentAction}
                          onClick={() => {
                            void handlePayment();
                          }}
                          type="button"
                        >
                          {isSubmittingPayment
                            ? "Submitting payment..."
                            : "Pay with wallet"}
                        </button>
                      ) : null}
                      {showRetryAction ? (
                        <button
                          className="spawn-nav-button"
                          disabled={spawnSession.isMutating}
                          onClick={() => {
                            void spawnSession.retry();
                          }}
                          type="button"
                        >
                          Retry spawn
                        </button>
                      ) : null}
                      {showRefundAction ? (
                        <button
                          className="spawn-nav-button"
                          disabled={spawnSession.isMutating}
                          onClick={() => {
                            void spawnSession.refund();
                          }}
                          type="button"
                        >
                          Claim refund
                        </button>
                      ) : null}
                    </div>
                  ) : null}

                  {showPaymentAction ? (
                    <p className="spawn-session-meta">
                      {paymentActionDisabledReason ??
                        "This submits a USDC approval followed by the escrow deposit transaction from the connected wallet. Wallets may still show 0 ETH because the payment value is carried in contract calldata, not as native ETH."}
                    </p>
                  ) : null}

                  {activeSession.state === "awaiting_payment" &&
                  paymentResult === null ? (
                    <p className="spawn-session-error" role="alert">
                      No playground payment has been detected for this session yet.
                      If you already clicked pay, open the wallet notification
                      queue and confirm both the USDC approval and the escrow
                      deposit on {playgroundChainName} (chain{" "}
                      {playgroundMetadata?.chain.id ?? "pending"}).
                    </p>
                  ) : null}

                  {activeSession.state === "awaiting_payment" &&
                  activeSession.paymentStatus === "unpaid" &&
                  paymentResult !== null &&
                  effectivePendingReceipts.length === 0 ? (
                    <p className="spawn-session-meta">
                      Wallet transactions were confirmed on {playgroundChainName}{" "}
                      (chain {playgroundMetadata?.chain.id ?? "pending"}), but
                      the factory has not marked this session as paid yet. Leave
                      this session open while the indexer catches up.
                    </p>
                  ) : null}

                  {activeSession.state === "awaiting_payment" &&
                  paymentResult !== null &&
                  effectivePendingReceipts.length > 0 ? (
                    <p className="spawn-session-meta">
                      Wallet transactions were submitted and are still waiting
                      for on-chain confirmation. Leave this session open while
                      the playground confirms the{" "}
                      {describePendingReceipts(effectivePendingReceipts)}.
                    </p>
                  ) : null}

                  <p className="spawn-session-meta">
                    {spawnSession.isCreating
                      ? "Creating factory session reference."
                      : spawnSession.isMutating
                        ? "Submitting factory session action."
                        : spawnSession.isRefreshing
                          ? "Refreshing factory session state from the indexer."
                          : activeSession.state === "awaiting_payment" &&
                              paymentResult !== null &&
                              effectivePendingReceipts.length === 0
                            ? "Waiting for the factory/indexer to mirror the confirmed playground payment."
                            : "Live spawn progress updates automatically as indexer events arrive."}
                  </p>

                  {activeSession.state === "expired" ? (
                    <p className="spawn-session-error" role="alert">
                      This session expired before completion. The quote TTL may
                      have elapsed or the playground may have reset. Start a new
                      session and claim a refund first if one is available.
                    </p>
                  ) : null}

                  {playgroundMetadata?.maintenance ? (
                    <p className="spawn-session-error" role="alert">
                      Playground maintenance is active. New sessions are paused
                      until the reset completes.
                    </p>
                  ) : null}

                  {paymentResult !== null ? (
                    <p className="spawn-session-meta">
                      {effectivePendingReceipts.length === 0
                        ? "Transactions submitted and confirmed."
                        : "Transactions submitted."}{" "}
                      Approval tx: {formatShortHash(paymentResult.approvalTxHash)}
                      . Deposit tx: {formatShortHash(paymentResult.paymentTxHash)}.
                      {effectivePendingReceipts.length > 0
                        ? ` Waiting for ${describePendingReceipts(
                            effectivePendingReceipts
                          )} confirmation.`
                        : ""}
                    </p>
                  ) : null}
                </>
              ) : (
                <p className="spawn-session-meta">
                  Session reference details will appear here as soon as the
                  factory returns the live quote and escrow instructions.
                </p>
              )}

              {paymentError !== null ? (
                <p className="spawn-session-error" role="alert">
                  {paymentError}
                </p>
              ) : null}

              {spawnSession.error !== null ? (
                <p className="spawn-session-error" role="alert">
                  {spawnSession.error}
                </p>
              ) : null}
            </section>
          ) : null}
        </div>

        <footer className="spawn-footer">
          {hasTrackedSession ? (
            <>
              <button className="spawn-nav-button" onClick={closeWizard} type="button">
                Close
              </button>
              <button
                className="spawn-nav-button is-primary"
                disabled={spawnSession.isCreating || spawnSession.isMutating}
                onClick={resetTrackedSession}
                type="button"
              >
                New session
              </button>
            </>
          ) : (
            <>
              <button
                className="spawn-nav-button"
                disabled={stepIndex === 0 || spawnSession.isCreating || isSubmittingPayment}
                onClick={retreatStep}
                type="button"
              >
                Back
              </button>
              <button
                className="spawn-nav-button is-primary"
                disabled={stepIndex === TOTAL_SPAWN_STEPS - 1 ? !canSubmit : false}
                onClick={
                  stepIndex === TOTAL_SPAWN_STEPS - 1
                    ? () => {
                        void handleSubmit();
                      }
                    : advanceStep
                }
                type="button"
              >
                {stepIndex === TOTAL_SPAWN_STEPS - 1 ? "Spawn" : "Next"}
              </button>
            </>
          )}
        </footer>
      </section>
    </div>
  );
}
