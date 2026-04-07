function parseHexInteger(value: unknown): number | null {
  if (typeof value !== "string" || !/^0x[0-9a-f]+$/iu.test(value)) {
    return null;
  }

  return Number.parseInt(value, 16);
}

function padAddress(address: string) {
  return address.toLowerCase().replace(/^0x/u, "").padStart(64, "0");
}

export interface EvmObservation {
  ethBalanceWei: string | null;
  usdcBalanceRaw: string | null;
  txCount: number | null;
}

export class EvmClient {
  constructor(
    private readonly rpcUrl: string,
    private readonly fetchImpl: typeof fetch = fetch
  ) {}

  private async rpc<T>(method: string, params: unknown[]): Promise<T> {
    const response = await this.fetchImpl(this.rpcUrl, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        accept: "application/json"
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
        message?: string;
      };
      result?: T;
    };

    if (!response.ok || payload.error) {
      throw new Error(payload.error?.message ?? `${method} failed.`);
    }

    return payload.result as T;
  }

  async observeAddress(address: string | null, usdcAddress: string | null): Promise<EvmObservation> {
    if (address === null) {
      return {
        ethBalanceWei: null,
        usdcBalanceRaw: null,
        txCount: null
      };
    }

    const [ethBalanceHex, txCountHex, usdcBalanceHex] = await Promise.all([
      this.rpc<string>("eth_getBalance", [address, "latest"]),
      this.rpc<string>("eth_getTransactionCount", [address, "latest"]),
      usdcAddress
        ? this.rpc<string>("eth_call", [
            {
              to: usdcAddress,
              data: `0x70a08231${padAddress(address)}`
            },
            "latest"
          ])
        : Promise.resolve("0x0")
    ]);

    return {
      ethBalanceWei:
        typeof ethBalanceHex === "string" && /^0x[0-9a-f]+$/iu.test(ethBalanceHex)
          ? BigInt(ethBalanceHex).toString()
          : null,
      usdcBalanceRaw:
        typeof usdcBalanceHex === "string" && /^0x[0-9a-f]+$/iu.test(usdcBalanceHex)
          ? BigInt(usdcBalanceHex).toString()
          : null,
      txCount: parseHexInteger(txCountHex)
    };
  }
}

export interface EvmClientLike {
  observeAddress(address: string | null, usdcAddress: string | null): Promise<EvmObservation>;
}
