export declare const PLAYGROUND_FAUCET_ASSETS: readonly ["eth", "usdc"];
export type PlaygroundFaucetAsset = (typeof PLAYGROUND_FAUCET_ASSETS)[number];
export interface PlaygroundNativeCurrency {
    name: string;
    symbol: string;
    decimals: number;
}
export interface PlaygroundClaimLimits {
    windowSeconds: number;
    maxClaimsPerWallet: number;
    maxClaimsPerIp: number;
}
export interface PlaygroundClaimAssetAmount {
    asset: PlaygroundFaucetAsset;
    amount: string;
    decimals: number;
}
export interface PlaygroundMetadata {
    environmentLabel: string;
    environmentVersion: string | null;
    maintenance: boolean;
    chain: {
        id: number;
        name: string;
        publicRpcUrl: string;
        nativeCurrency: PlaygroundNativeCurrency;
        explorerUrl: string | null;
    };
    faucet: {
        available: boolean;
        claimLimits: PlaygroundClaimLimits;
        claimAssetAmounts: PlaygroundClaimAssetAmount[];
    };
    reset: {
        lastResetAt: number | null;
        nextResetAt: number | null;
        cadenceLabel: string;
    };
}
//# sourceMappingURL=playground.d.ts.map