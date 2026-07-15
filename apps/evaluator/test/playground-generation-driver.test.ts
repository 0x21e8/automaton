import { describe, expect, it } from "vitest";
import { createHash } from "node:crypto";

import { IndexerHttpError } from "../src/lib/indexer-client.js";
import { PlaygroundGenerationScenarioDriver } from "../src/lib/playground-generation-driver.js";

const runtime = {
  paymentRpcUrl: "http://127.0.0.1:8545",
  usdcAddress: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
  chainId: 20_260_326
};
const evaluationEnv = {
  PLAYGROUND_ICP_ENVIRONMENT: "local",
  LOCAL_EVM_RPC_URL: "http://127.0.0.1:8545"
};

describe("playground generation driver authority path", () => {
  function sha256(value: string) {
    return createHash("sha256").update(value).digest("hex");
  }

  it("mints only on the exact evaluation deployment and verifies indexed balance delta", async () => {
    const commands: string[][] = [];
    let reads = 0;
    let chainReads = 0;
    const indexer = {
      async fetchAutomatonDetail() {
        reads += 1;
        return { ethAddress: "0xchild", financials: { usdcBalanceRaw: reads === 1 ? "100" : "35" } };
      }
    };
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", runtime as never, indexer as never, evaluationEnv,
      async (_command, args) => {
        commands.push(args);
        if (args[0] === "call") {
          chainReads += 1;
          return { stdout: chainReads === 1 ? "10" : "35" };
        }
        return { stdout: JSON.stringify({ status: "0x1" }) };
      }
    );

    await expect(driver.seedEarnings("child-cai", "25")).resolves.toBeUndefined();
    expect(commands[0]?.slice(0, 4)).toEqual(["call", expect.stringMatching(/^0x833589/i), "balanceOf(address)(uint256)", "0xchild"]);
    expect(commands[1]?.slice(0, 5)).toEqual(["send", expect.stringMatching(/^0x833589/i), "mint(address,uint256)", "0xchild", "25"]);
    expect(commands[2]?.slice(0, 4)).toEqual(["call", expect.stringMatching(/^0x833589/i), "balanceOf(address)(uint256)", "0xchild"]);
    expect(reads).toBe(2);
    await expect(driver.readLineageMetrics()).resolves.toMatchObject({
      earningsSeeds: [{ indexedBefore: "100", chainBefore: "10", chainAfter: "35", indexedAfter: "35" }]
    });
  });

  it("refuses evaluation minting outside the exact local chain context", async () => {
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", { ...runtime, chainId: 8453 } as never, {} as never,
      evaluationEnv, async () => ({ stdout: JSON.stringify({ status: "0x1" }) })
    );
    await expect(driver.seedEarnings("child-cai", "25")).rejects.toThrow("restricted to the exact local generations deployment");
  });

  it("uses the direct local EVM RPC for privileged time travel instead of the public gateway", async () => {
    const commands: string[][] = [];
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", { ...runtime, paymentRpcUrl: "http://127.0.0.1:3002" } as never, {} as never,
      { PLAYGROUND_ICP_ENVIRONMENT: "local", LOCAL_EVM_RPC_URL: "http://127.0.0.1:9545" },
      async (_command, args) => { commands.push(args); return { stdout: "ok" }; }
    );
    await driver.advanceTime(1_000);
    expect(commands[1]).toContain("http://127.0.0.1:9545");
    expect(commands[2]).toContain("http://127.0.0.1:9545");
    expect(commands.flat()).not.toContain("http://127.0.0.1:3002");
  });

  it("never falls back to the public gateway for evaluation mutations", async () => {
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", { ...runtime, paymentRpcUrl: "http://127.0.0.1:3002" } as never, {} as never,
      { PLAYGROUND_ICP_ENVIRONMENT: "local" }, async () => ({ stdout: "ok" })
    );
    await expect(driver.advanceTime(1_000)).rejects.toThrow("explicit direct LOCAL_EVM_RPC_URL");
  });

  it("rejects a failed mint receipt before accepting any indexed balance", async () => {
    const indexer = { async fetchAutomatonDetail() { return { ethAddress: "0xchild", financials: { usdcBalanceRaw: "100" } }; } };
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", runtime as never, indexer as never, evaluationEnv,
      async (_command, args) => args[0] === "call" ? ({ stdout: "10" }) : ({ stdout: JSON.stringify({ status: "0x0" }) })
    );
    await expect(driver.seedEarnings("child-cai", "25")).rejects.toThrow("receipt was not successful");
  });

  it("seeds earnings from a zero authoritative balance after payment settlement is verified separately", async () => {
    let indexedReads = 0;
    let chainReads = 0;
    const indexer = { async fetchAutomatonDetail() {
      indexedReads += 1;
      return { ethAddress: "0xchild", financials: { usdcBalanceRaw: indexedReads === 1 ? "0" : "25" } };
    } };
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", runtime as never, indexer as never, evaluationEnv,
      async (_command, args) => {
        if (args[0] === "call") {
          chainReads += 1;
          return { stdout: chainReads === 1 ? "0" : "25" };
        }
        if (args[0] === "send") return { stdout: JSON.stringify({ status: "0x1" }) };
        return { stdout: "(variant { Ok = \"ok\" })" };
      }
    );
    await expect(driver.seedEarnings("child-cai", "25")).resolves.toBeUndefined();
    await expect(driver.readLineageMetrics()).resolves.toMatchObject({
      earningsSeeds: [{ indexedBefore: "0", chainBefore: "0", chainAfter: "25", indexedAfter: "25" }]
    });
  });

  it("waits for delayed child indexing, then records factory-routed inheritance and starvation evidence", async () => {
    const commands: string[][] = [];
    let parentReads = 0;
    let childReads = 0;
    const parentConstitution = "I am a patient observer. ".repeat(20);
    const parent = {
      constitution: parentConstitution,
      constitutionHash: sha256(parentConstitution),
      constitutionVerification: { status: "verified" },
      childIds: [] as string[],
      generation: 0,
      metabolism: { deathCause: null, diedAt: null }
    };
    const childConstitution = parent.constitution.replace(/\bpatient\b/i, "deliberate");
    const indexer = {
      async fetchAutomatonDetail(canisterId: string) {
        if (canisterId === "parent-cai") {
          parentReads += 1;
          return parentReads > 1 ? { ...parent, childIds: ["child-cai"] } : parent;
        }
        childReads += 1;
        if (childReads === 1) throw new IndexerHttpError(404, "Automaton not found");
      return {
        ...parent,
        constitution: childConstitution,
        constitutionHash: sha256(childConstitution),
          parentConstitutionHash: parent.constitutionHash,
          parentId: "parent-cai",
          childIds: [],
          generation: 1,
          constitutionVerification: { status: childReads === 2 ? "pending" : "verified" },
          metabolism: { deathCause: "starved", diedAt: 42 }
        };
      }
    };
    const driver = new PlaygroundGenerationScenarioDriver(
      "/repo", runtime as never, indexer as never, evaluationEnv,
      async (_command, args) => {
        commands.push(args);
        if (args.includes("list_memory_facts")) return { stdout: "inherited.dowry.evaluation.dowry.market_regime range-bound-liquidity genesis:inherited" };
        return { stdout: "(variant { Ok = \"ok\" })" };
      }
    );

    await expect(driver.reproduce("parent-cai", "child-name")).resolves.toEqual({ childCanisterId: "child-cai" });
    await expect(driver.starve("child-cai")).resolves.toBeUndefined();

    expect(commands[0]?.slice(0, 5)).toEqual(["canister", "call", "factory", "run_evaluation_seed_memory", expect.stringContaining("parent-cai")]);
    expect(commands[1]?.slice(0, 5)).toEqual(["canister", "call", "factory", "run_evaluation_reproduction", expect.stringContaining("parent-cai")]);
    expect(commands[2]).toContain("list_memory_facts");
    expect(commands[3]?.slice(0, 5)).toEqual(["canister", "call", "factory", "run_evaluation_starvation", expect.stringContaining("child-cai")]);
    expect(childReads).toBeGreaterThanOrEqual(2);
    await expect(driver.readLineageMetrics()).resolves.toMatchObject({
      inheritance: [{
        childCanisterId: "child-cai",
        parentConstitutionHash: sha256(parentConstitution),
        childConstitutionHash: sha256(parent.constitution.replace(/\bpatient\b/i, "deliberate"))
      }]
    });
  });
});
