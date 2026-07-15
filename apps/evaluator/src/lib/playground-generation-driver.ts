import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";

import type { PlaygroundRuntime } from "../types.js";
import { IndexerHttpError, type IndexerClientLike } from "./indexer-client.js";
import type { GenerationScenarioDriver } from "./generation-scenario.js";
import type { InheritanceEvidence, SeedEarningsEvidence } from "./generation-scenario.js";

const execFileAsync = promisify(execFile);
const DEFAULT_ANVIL_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const EVALUATION_CHAIN_ID = 20_260_326;
const CANONICAL_BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const EVALUATION_MEMORY_KEY = "evaluation.dowry.market_regime";
const EVALUATION_MEMORY_VALUE = "range-bound-liquidity";
const STARVATION_TIMEOUT_MS = 300_000;
const VERIFICATION_RETRY_MS = 2_000;
const REPRODUCTION_TIMEOUT_MS = 180_000;

function candidText(value: string) {
  return JSON.stringify(value);
}

function sha256(value: string) {
  return createHash("sha256").update(value).digest("hex");
}

export class PlaygroundGenerationScenarioDriver implements GenerationScenarioDriver {
  private readonly descendants = new Map<string, string>();
  private readonly starved = new Set<string>();
  private readonly inheritance: InheritanceEvidence[] = [];
  private readonly earningsSeeds: SeedEarningsEvidence[] = [];

  constructor(
    private readonly repoRoot: string,
    private readonly runtime: PlaygroundRuntime,
    private readonly indexer: IndexerClientLike,
    private readonly env: NodeJS.ProcessEnv = process.env,
    private readonly commandRunner?: (command: string, args: string[]) => Promise<unknown>
  ) {}

  private async command(command: string, args: string[]) {
    if (this.commandRunner !== undefined) return this.commandRunner(command, args);
    return execFileAsync(command, args, { cwd: this.repoRoot, env: this.env, maxBuffer: 4 * 1024 * 1024 });
  }

  private commandStdout(result: unknown) {
    if (typeof result === "string") return result;
    if (typeof result === "object" && result !== null && "stdout" in result && typeof result.stdout === "string") return result.stdout;
    return "";
  }

  private ensureLocalEvaluationMint() {
    const hostname = new URL(this.paymentRpcUrl()).hostname;
    const environment = this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local";
    if (
      this.runtime.chainId !== EVALUATION_CHAIN_ID ||
      environment !== "local" ||
      !["127.0.0.1", "localhost", "::1"].includes(hostname) ||
      this.runtime.usdcAddress.toLowerCase() !== CANONICAL_BASE_USDC
    ) {
      throw new Error("evaluation USDC mint is restricted to the exact local generations deployment");
    }
  }

  private paymentRpcUrl() {
    const directRpcUrl = this.env.LOCAL_EVM_RPC_URL?.trim();
    if (!directRpcUrl) {
      throw new Error("generation evaluation mutations require an explicit direct LOCAL_EVM_RPC_URL");
    }
    const hostname = new URL(directRpcUrl).hostname;
    if (!["127.0.0.1", "localhost", "::1"].includes(hostname)) {
      throw new Error("generation evaluation mutations require a loopback LOCAL_EVM_RPC_URL");
    }
    return directRpcUrl;
  }

  async seedEarnings(canisterId: string, amountRaw: string) {
    this.ensureLocalEvaluationMint();
    let detail = await this.indexer.fetchAutomatonDetail(canisterId);
    if (detail.ethAddress === null) throw new Error(`automaton ${canisterId} has no EVM address`);
    const balanceResult = await this.command("cast", [
      "call", this.runtime.usdcAddress, "balanceOf(address)(uint256)", detail.ethAddress,
      "--rpc-url", this.paymentRpcUrl()
    ]);
    const balanceToken = this.commandStdout(balanceResult).trim().split(/\s+/)[0];
    if (balanceToken === undefined || !/^(?:0x[0-9a-f]+|[0-9]+)$/i.test(balanceToken)) {
      throw new Error("evaluation USDC balanceOf returned malformed data");
    }
    const chainBefore = BigInt(balanceToken);
    const indexedBefore = BigInt(detail.financials.usdcBalanceRaw ?? "0");
    const receipt = await this.command("cast", [
      "send", this.runtime.usdcAddress, "mint(address,uint256)", detail.ethAddress, amountRaw,
      "--private-key", this.env.PLAYGROUND_ANVIL_PRIVATE_KEY?.trim() || DEFAULT_ANVIL_PRIVATE_KEY,
      "--rpc-url", this.paymentRpcUrl(), "--json"
    ]);
    const receiptText = this.commandStdout(receipt);
    let status: unknown;
    try { status = (JSON.parse(receiptText) as { status?: unknown }).status; } catch { status = undefined; }
    if (status !== "0x1" && status !== "success" && status !== 1) {
      throw new Error(`evaluation USDC mint receipt was not successful: ${receiptText || "missing receipt"}`);
    }
    const expected = chainBefore + BigInt(amountRaw);
    const afterResult = await this.command("cast", [
      "call", this.runtime.usdcAddress, "balanceOf(address)(uint256)", detail.ethAddress,
      "--rpc-url", this.paymentRpcUrl()
    ]);
    const afterToken = this.commandStdout(afterResult).trim().split(/\s+/)[0];
    if (afterToken === undefined || !/^(?:0x[0-9a-f]+|[0-9]+)$/i.test(afterToken) || BigInt(afterToken) < expected) {
      throw new Error(`evaluation USDC mint did not increase the authoritative balance by ${amountRaw}`);
    }
    const chainAfter = BigInt(afterToken);
    await this.command("icp", [
      "canister", "call", "factory", "run_evaluation_wallet_balance_sync",
      `(${candidText(canisterId)})`, "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
    ]);
    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      const refreshed = await this.indexer.fetchAutomatonDetail(canisterId);
      const indexedAfter = BigInt(refreshed.financials.usdcBalanceRaw ?? "0");
      if (indexedAfter >= chainAfter) {
        this.earningsSeeds.push({
          canisterId,
          evmAddress: detail.ethAddress,
          indexedBefore: indexedBefore.toString(),
          chainBefore: chainBefore.toString(),
          amountRaw,
          chainAfter: chainAfter.toString(),
          indexedAfter: indexedAfter.toString()
        });
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new Error(`timed out waiting for seeded earnings on ${canisterId}`);
  }

  async advanceTime(durationMs: number) {
    const paymentRpcUrl = this.paymentRpcUrl();
    await this.command("icp", [
      "canister", "call", "factory", "advance_evaluation_time",
      `(${durationMs} : nat64)`, "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
    ]);
    // Keep EVM timestamps coherent for receipts and log polling.
    await this.command("cast", ["rpc", "evm_increaseTime", String(Math.ceil(durationMs / 1000)), "--rpc-url", paymentRpcUrl]);
    await this.command("cast", ["rpc", "evm_mine", "--rpc-url", paymentRpcUrl]);
  }

  async reproduce(parentCanisterId: string, childId: string) {
    const parentDeadline = Date.now() + REPRODUCTION_TIMEOUT_MS;
    let parent = await this.indexer.fetchAutomatonDetail(parentCanisterId);
    while (
      Date.now() < parentDeadline &&
      (parent.constitution == null || parent.constitutionHash == null)
    ) {
      await new Promise((resolve) => setTimeout(resolve, VERIFICATION_RETRY_MS));
      parent = await this.indexer.fetchAutomatonDetail(parentCanisterId);
    }
    if (parent.constitution == null || parent.constitutionHash == null) {
      throw new Error(`parent ${parentCanisterId} constitution hash data was not available`);
    }
    if (parent.constitutionVerification?.status === "mismatch") {
      throw new Error(`parent ${parentCanisterId} constitution hash did not match registry hash`);
    }
    await this.command("icp", [
      "canister", "call", "factory", "run_evaluation_seed_memory",
      `(${candidText(parentCanisterId)}, ${candidText(EVALUATION_MEMORY_KEY)}, ${candidText(EVALUATION_MEMORY_VALUE)})`,
      "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
    ]);
    const parentConstitution = parent.constitution;
    const childConstitution = parentConstitution.replace(/\bpatient\b/i, "deliberate");
    const childConstitutionCandidates = [
      childConstitution,
      `${parentConstitution} I carry one new observation.`
    ];
    const expectedChildHashes = childConstitutionCandidates.map((value) => sha256(value).toLowerCase());
    const argsJson = JSON.stringify({
      name: childId,
      child_constitution: childConstitution === parentConstitution ? `${parentConstitution} I carry one new observation.` : childConstitution,
      gross_amount: "75000000",
      memory_dowry_keys: [EVALUATION_MEMORY_KEY]
    });
    const before = new Set(parent.childIds);
    await this.command("icp", [
      "canister", "call", "factory", "run_evaluation_reproduction",
      `(${candidText(parentCanisterId)}, ${candidText(argsJson)})`, "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
    ]);

    const deadline = Date.now() + 180_000;
    while (Date.now() < deadline) {
      const refreshed = await this.indexer.fetchAutomatonDetail(parentCanisterId);
      const created = refreshed.childIds.find((id) => !before.has(id));
      if (created !== undefined) {
        let child;
        try {
          child = await this.indexer.fetchAutomatonDetail(created);
        } catch (error) {
          if (error instanceof IndexerHttpError && error.status === 404) {
            await new Promise((resolve) => setTimeout(resolve, 2_000));
            continue;
          }
          throw error;
        }
        if (
          child.constitution == null || child.constitutionHash == null ||
          child.constitutionVerification?.status === "mismatch" ||
          child.parentId !== parentCanisterId || child.generation !== (parent.generation ?? 0) + 1 ||
          child.parentConstitutionHash !== parent.constitutionHash || child.constitutionHash === parent.constitutionHash ||
          !expectedChildHashes.includes(child.constitutionHash.toLowerCase())
        ) {
          await new Promise((resolve) => setTimeout(resolve, VERIFICATION_RETRY_MS));
          continue;
        }
        const memoryResult = await this.command("icp", [
          "canister", "call", created, "list_memory_facts",
          `("inherited.dowry.${EVALUATION_MEMORY_KEY}", variant { KeyAsc }, 10 : nat32)`,
          "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
        ]);
        const memoryOutput = this.commandStdout(memoryResult);
        const inheritedMemoryKey = `inherited.dowry.${EVALUATION_MEMORY_KEY}`;
        if (!memoryOutput.includes(inheritedMemoryKey) || !memoryOutput.includes(EVALUATION_MEMORY_VALUE) || !memoryOutput.includes("genesis:inherited")) {
          throw new Error(`descendant ${created} did not expose the requested inherited memory fact and genesis tag`);
        }
        const left = parentConstitution.split(/\s+/);
        const right = child.constitution.split(/\s+/);
        const constitutionDiff: string[] = [];
        for (let index = 0; index < Math.max(left.length, right.length) && constitutionDiff.length < 80; index += 1) {
          if (left[index] === right[index]) continue;
          if (left[index] !== undefined) constitutionDiff.push(`− ${left[index]}`);
          if (right[index] !== undefined) constitutionDiff.push(`+ ${right[index]}`);
        }
        if (constitutionDiff.length === 0) throw new Error(`descendant ${created} constitution diff is empty`);
        this.descendants.set(childId, created);
        this.inheritance.push({
          parentCanisterId,
          childCanisterId: created,
          parentConstitutionHash: parent.constitutionHash,
          childConstitutionHash: child.constitutionHash,
          childRecordedParentHash: child.parentConstitutionHash,
          generation: child.generation,
          memoryKey: EVALUATION_MEMORY_KEY,
          memoryValue: EVALUATION_MEMORY_VALUE,
          inheritedMemoryKey,
          inheritedSourceTag: "genesis:inherited",
          constitutionDiff
        });
        return { childCanisterId: created };
      }
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new Error(`timed out waiting for descendant of ${parentCanisterId}`);
  }

  async starve(canisterId: string) {
    await this.command("icp", [
      "canister", "call", "factory", "run_evaluation_starvation", `(${candidText(canisterId)})`,
      "-e", this.env.PLAYGROUND_ICP_ENVIRONMENT?.trim() || "local"
    ]);
    const deadline = Date.now() + STARVATION_TIMEOUT_MS;
    while (Date.now() < deadline) {
      const detail = await this.indexer.fetchAutomatonDetail(canisterId);
      if (detail.metabolism?.deathCause === "starved" && detail.metabolism.diedAt != null) {
        this.starved.add(canisterId);
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    throw new Error(`timed out waiting for authoritative starvation record on ${canisterId}`);
  }

  async readLineageMetrics() {
    let generationDepth = 0;
    for (const canisterId of this.descendants.values()) {
      const detail = await this.indexer.fetchAutomatonDetail(canisterId);
      generationDepth = Math.max(generationDepth, detail.generation ?? 0);
    }
    const survivingDescendantIds: string[] = [];
    for (const canisterId of this.descendants.values()) {
      const detail = await this.indexer.fetchAutomatonDetail(canisterId);
      if (detail.metabolism?.diedAt == null) survivingDescendantIds.push(canisterId);
    }
    return {
      descendantCount: this.descendants.size,
      generationDepth,
      starvedCanisterIds: [...this.starved],
      survivingDescendantIds
      , inheritance: this.inheritance,
      earningsSeeds: this.earningsSeeds
    };
  }
}
