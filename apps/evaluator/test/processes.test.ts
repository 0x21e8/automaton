import { describe, expect, it } from "vitest";

import { resolveUsdcAddressFromDeployment } from "../src/lib/processes.js";

describe("resolveUsdcAddressFromDeployment", () => {
  it("prefers usdcAddress when present", () => {
    expect(
      resolveUsdcAddressFromDeployment({
        usdcAddress: "0x111",
        usdcTokenAddress: "0x222",
        mockUsdcAddress: "0x333"
      })
    ).toBe("0x111");
  });

  it("falls back to usdcTokenAddress and mockUsdcAddress", () => {
    expect(
      resolveUsdcAddressFromDeployment({
        usdcTokenAddress: "0x222"
      })
    ).toBe("0x222");

    expect(
      resolveUsdcAddressFromDeployment({
        mockUsdcAddress: "0x333"
      })
    ).toBe("0x333");
  });

  it("returns an empty string when no deployment address exists", () => {
    expect(resolveUsdcAddressFromDeployment({})).toBe("");
  });
});
