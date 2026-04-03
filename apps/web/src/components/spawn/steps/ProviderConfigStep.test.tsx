import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { defaultModelOptions } from "../../../lib/default-models";
import { ProviderConfigStep } from "./ProviderConfigStep";

describe("ProviderConfigStep", () => {
  it("omits the manual override and catalog status copy", () => {
    const markup = renderToStaticMarkup(
      <ProviderConfigStep
        braveSearchApiKey=""
        modelOptions={defaultModelOptions}
        onBraveSearchApiKeyChange={vi.fn()}
        onOpenRouterApiKeyChange={vi.fn()}
        onSelectedModelChange={vi.fn()}
        openRouterApiKey=""
        selectedModelId=""
      />
    );

    expect(markup).toContain("Inference model");
    expect(markup).toContain("Brave Search API key");
    expect(markup).not.toContain("Manual model override");
    expect(markup).not.toContain("Loaded live OpenRouter models");
    expect(markup).not.toContain("Loading live OpenRouter models.");
    expect(markup).not.toContain(defaultModelOptions[0]?.description ?? "");
  });
});
