import { describe, expect, it } from "vitest";
import { fileURLToPath } from "node:url";

import { resolveIndexerConfig } from "../src/config.js";
import { INDEXER_TARGET_CONFIG } from "../src/indexer.config.js";
import { buildServer, formatStartupError } from "../src/server.js";

const defaultPlaygroundStatusFilePath = fileURLToPath(
  new URL("../tmp/playground-status.json", import.meta.url)
);

describe("indexer config", () => {
  it("loads only explicit normalized settlement trust anchors and otherwise fails closed", () => {
    expect(resolveIndexerConfig({}).society).toEqual({ trustedInboxAddresses: [], trustedUsdcAddresses: [] });
    expect(resolveIndexerConfig({
      INDEXER_TRUSTED_INBOX_ADDRESSES: "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,invalid,0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      INDEXER_TRUSTED_USDC_ADDRESSES: "0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
    }).society).toEqual({
      trustedInboxAddresses: ["0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
      trustedUsdcAddresses: ["0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]
    });
  });

  it("uses the dedicated target config file as the default ingestion source", () => {
    const config = resolveIndexerConfig({});

    expect(config.ingestion).toEqual(INDEXER_TARGET_CONFIG);
    expect(config.icHost).toBe("http://localhost:8000");
    expect(config.corsAllowedOrigins).toEqual([
      "http://127.0.0.1:5173",
      "http://localhost:5173"
    ]);
    expect(config.playground).toEqual({
      metadata: {
        environmentLabel: "Local development",
        environmentVersion: null,
        maintenance: false,
        chain: {
          id: 8453,
          name: "Base Local Fork",
          publicRpcUrl: "http://127.0.0.1:8545",
          nativeCurrency: {
            name: "Ether",
            symbol: "ETH",
            decimals: 18
          },
          explorerUrl: null
        },
        faucet: {
          available: false,
          claimLimits: {
            windowSeconds: 86_400,
            maxClaimsPerWallet: 1,
            maxClaimsPerIp: 1
          },
          claimAssetAmounts: [
            {
              asset: "eth",
              amount: "1",
              decimals: 18
            },
            {
              asset: "usdc",
              amount: "250",
              decimals: 6
            }
          ]
        },
        reset: {
          lastResetAt: null,
          nextResetAt: null,
          cadenceLabel: "Manual local resets"
        }
      },
      statusFilePath: defaultPlaygroundStatusFilePath
    });
  });

  it("parses explicit playground runtime metadata from env", () => {
    const config = resolveIndexerConfig({
      PLAYGROUND_ENV_LABEL: "Automaton Playground",
      PLAYGROUND_ENV_VERSION: "2026.03.26+sha.abcdef",
      PLAYGROUND_MAINTENANCE: "true",
      PLAYGROUND_CHAIN_ID: "20260326",
      PLAYGROUND_CHAIN_NAME: "Automaton Playground",
      PLAYGROUND_PUBLIC_RPC_URL: "https://rpc.playground.example.com",
      PLAYGROUND_NATIVE_CURRENCY_NAME: "Play Ether",
      PLAYGROUND_NATIVE_CURRENCY_SYMBOL: "pETH",
      PLAYGROUND_NATIVE_CURRENCY_DECIMALS: "18",
      PLAYGROUND_EXPLORER_URL: "https://otter.playground.example.com",
      PLAYGROUND_FAUCET_ENABLED: "1",
      PLAYGROUND_FAUCET_CLAIM_WINDOW_SECONDS: "43200",
      PLAYGROUND_FAUCET_MAX_CLAIMS_PER_WALLET: "2",
      PLAYGROUND_FAUCET_MAX_CLAIMS_PER_IP: "5",
      PLAYGROUND_FAUCET_ETH_AMOUNT: "0.25",
      PLAYGROUND_FAUCET_USDC_AMOUNT: "75",
      PLAYGROUND_LAST_RESET_AT: "2026-03-26T10:00:00Z",
      PLAYGROUND_NEXT_RESET_AT: "2026-03-27T10:00:00Z",
      PLAYGROUND_RESET_CADENCE_LABEL: "Daily at 10:00 UTC",
      PLAYGROUND_STATUS_FILE: "/srv/automaton/playground-status.json"
    });

    expect(config.playground).toEqual({
      metadata: {
        environmentLabel: "Automaton Playground",
        environmentVersion: "2026.03.26+sha.abcdef",
        maintenance: true,
        chain: {
          id: 20_260_326,
          name: "Automaton Playground",
          publicRpcUrl: "https://rpc.playground.example.com",
          nativeCurrency: {
            name: "Play Ether",
            symbol: "pETH",
            decimals: 18
          },
          explorerUrl: "https://otter.playground.example.com"
        },
        faucet: {
          available: true,
          claimLimits: {
            windowSeconds: 43_200,
            maxClaimsPerWallet: 2,
            maxClaimsPerIp: 5
          },
          claimAssetAmounts: [
            {
              asset: "eth",
              amount: "0.25",
              decimals: 18
            },
            {
              asset: "usdc",
              amount: "75",
              decimals: 6
            }
          ]
        },
        reset: {
          lastResetAt: Date.parse("2026-03-26T10:00:00Z"),
          nextResetAt: Date.parse("2026-03-27T10:00:00Z"),
          cadenceLabel: "Daily at 10:00 UTC"
        }
      },
      statusFilePath: "/srv/automaton/playground-status.json"
    });
  });

  it("allows deployment-time env overrides for network targeting without replacing canister IDs", () => {
    const config = resolveIndexerConfig({
      INDEXER_INGESTION_NETWORK_TARGET: "mainnet",
      INDEXER_INGESTION_LOCAL_HOST: "replica.internal",
      INDEXER_INGESTION_LOCAL_PORT: "4943"
    });

    expect(config.ingestion.canisterIds).toEqual(INDEXER_TARGET_CONFIG.canisterIds);
    expect(config.ingestion.network.target).toBe("mainnet");
    expect(config.ingestion.network.local.host).toBe("replica.internal");
    expect(config.ingestion.network.local.port).toBe(4943);
    expect(config.icHost).toBe("https://ic0.app");
  });

  it("allows deployment-time env overrides for seed canister ids", () => {
    const config = resolveIndexerConfig({
      INDEXER_INGESTION_CANISTER_IDS:
        "ryjl3-tyaaa-aaaaa-aaaba-cai, txyno-ch777-77776-aaaaq-cai"
    });

    expect(config.ingestion.canisterIds).toEqual([
      "ryjl3-tyaaa-aaaaa-aaaba-cai",
      "txyno-ch777-77776-aaaaq-cai"
    ]);
  });

  it("derives the IC host from the configured network target", () => {
    const config = resolveIndexerConfig(
      {},
      {
        ingestion: {
          canisterIds: ["aaaaa-aa"],
          network: {
            target: "mainnet",
            local: {
              host: "127.0.0.1",
              port: 4943
            }
          }
        }
      }
    );

    expect(config.ingestion.canisterIds).toEqual(["aaaaa-aa"]);
    expect(config.icHost).toBe("https://ic0.app");
  });

  it("applies local replica env overrides to the derived IC host", () => {
    const config = resolveIndexerConfig({
      INDEXER_INGESTION_LOCAL_HOST: "127.0.0.1",
      INDEXER_INGESTION_LOCAL_PORT: "4943"
    });

    expect(config.ingestion.network.target).toBe("local");
    expect(config.icHost).toBe("http://127.0.0.1:4943");
  });

  it("accepts deployment-time CORS origin overrides", () => {
    const config = resolveIndexerConfig({
      INDEXER_CORS_ALLOWED_ORIGINS: "https://app.example.com, http://127.0.0.1:4173"
    });

    expect(config.corsAllowedOrigins).toEqual([
      "https://app.example.com",
      "http://127.0.0.1:4173"
    ]);
  });

  it("fails when the canister list is empty", () => {
    expect(() =>
      resolveIndexerConfig(
        {},
        {
          ingestion: {
            canisterIds: [],
            network: {
              target: "local",
              local: {
                host: "localhost",
                port: 8000
              }
            }
          }
        }
      )
    ).toThrowError(
      "Indexer ingestion config must include at least one canister ID when factory discovery is not configured."
    );
  });

  it("allows an empty seed list when factory discovery is configured", () => {
    const config = resolveIndexerConfig(
      {},
      {
        ingestion: {
          canisterIds: [],
          network: {
            target: "local",
            local: {
              host: "localhost",
              port: 8000
            }
          }
        },
        factoryCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai"
      }
    );

    expect(config.ingestion.canisterIds).toEqual([]);
    expect(config.factoryCanisterId).toBe("ryjl3-tyaaa-aaaaa-aaaba-cai");
  });

  it("fails when a canister ID format is invalid", () => {
    expect(() =>
      resolveIndexerConfig(
        {},
        {
          ingestion: {
            canisterIds: ["not-a-canister-id"],
            network: {
              target: "local",
              local: {
                host: "localhost",
                port: 8000
              }
            }
          }
        }
      )
    ).toThrowError("Indexer ingestion config canisterIds[0] must be a valid canister ID.");
  });

  it("fails when the network target is unsupported", () => {
    expect(() =>
      resolveIndexerConfig(
        {
          INDEXER_INGESTION_NETWORK_TARGET: "staging"
        },
        {
          ingestion: {
            canisterIds: ["aaaaa-aa"],
            network: {
              target: "mainnet",
              local: {
                host: "localhost",
                port: 8000
              }
            }
          }
        }
      )
    ).toThrowError('Indexer ingestion config network.target must be one of "mainnet", "local".');
  });

  it("fails when local mode is missing the local host or port", () => {
    expect(() =>
      resolveIndexerConfig(
        {
          INDEXER_INGESTION_LOCAL_HOST: "",
          INDEXER_INGESTION_LOCAL_PORT: "0"
        },
        {
          ingestion: {
            canisterIds: ["aaaaa-aa"],
            network: {
              target: "local",
              local: {
                host: "",
                port: 0
              }
            }
          }
        }
      )
    ).toThrowError('network.local.host must be set when network.target is "local".');
  });

  it("fails server startup immediately when the ingestion config is invalid", () => {
    expect(() =>
      buildServer({
        config: {
          ingestion: {
            canisterIds: [],
            network: {
              target: "local",
              local: {
                host: "localhost",
                port: 8000
              }
            }
          }
        }
      })
    ).toThrowError(
      "Indexer ingestion config must include at least one canister ID when factory discovery is not configured."
    );
  });

  it("formats invalid ingestion startup failures with config guidance", () => {
    const error = new Error(
      'Invalid indexer ingestion config:\n- Indexer ingestion config network.target must be one of "mainnet", "local". Received: "staging".'
    );

    expect(formatStartupError(error)).toContain(
      "Indexer startup aborted: invalid ingestion target configuration."
    );
    expect(formatStartupError(error)).toContain("apps/indexer/src/indexer.config.ts");
    expect(formatStartupError(error)).toContain("INDEXER_INGESTION_NETWORK_TARGET");
    expect(formatStartupError(error)).toContain(
      'Indexer ingestion config network.target must be one of "mainnet", "local".'
    );
  });
});
