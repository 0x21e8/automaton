import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchOpenRouterModels } from "./openrouter";

describe("fetchOpenRouterModels", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requests text-output models, filters to text IO, uses curated popularity order, and hydrates live pricing", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
      return new Response(
        JSON.stringify({
          data: [
            {
              id: "custom/text-model",
              name: "Custom Text Model",
              description: "A text model.",
              architecture: {
                input_modalities: ["text"],
                output_modalities: ["text"]
              }
            },
            {
              id: "custom/image-model",
              name: "Custom Image Model",
              description: "An image model.",
              architecture: {
                input_modalities: ["text"],
                output_modalities: ["image"]
              }
            },
            {
              id: "stepfun/step-3.5-flash:free",
              name: "Step 3.5 Flash",
              description: "Dynamic catalog copy.",
              pricing: {
                prompt: "0",
                completion: "0",
                request: "0"
              },
              architecture: {
                input_modalities: ["text"],
                output_modalities: ["text"]
              }
            },
            {
              id: "minimax/minimax-m2.5:free",
              name: "MiniMax M2.5",
              description: "Dynamic pricing data.",
              pricing: {
                prompt: "0.0000012",
                completion: "0.0000048",
                request: "0"
              },
              architecture: {
                input_modalities: ["text"],
                output_modalities: ["text"]
              }
            }
          ]
        }),
        {
          status: 200,
          headers: {
            "content-type": "application/json"
          }
        }
      );
    });

    const models = await fetchOpenRouterModels();

    expect(fetchMock).toHaveBeenCalledWith(
      "https://openrouter.ai/api/v1/models?output_modalities=text",
      expect.objectContaining({
        headers: {
          Accept: "application/json"
        }
      })
    );
    expect(models.map((model) => model.id)).toContain("custom/text-model");
    expect(models.map((model) => model.id)).not.toContain("custom/image-model");
    expect(models.map((model) => model.id)).toContain("qwen/qwen3.6-plus-preview:free");
    expect(models.map((model) => model.id)).toContain("x-ai/grok-4.1-fast");
    expect(models[0]?.id).toBe("minimax/minimax-m2.5:free");
    expect(models[1]?.id).toBe("stepfun/step-3.5-flash:free");
    expect(models[0]?.pricing).toEqual({
      prompt: "0.0000012",
      completion: "0.0000048",
      request: "0"
    });
  });
});
