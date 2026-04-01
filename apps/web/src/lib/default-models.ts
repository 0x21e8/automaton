export type ProviderModelSource = "dynamic" | "fallback";

export interface ProviderModelPricing {
  completion: string | null;
  prompt: string | null;
  request: string | null;
}

export interface ProviderModelOption {
  id: string;
  label: string;
  description: string;
  pricing?: ProviderModelPricing;
  source: ProviderModelSource;
}

function createFallbackModel(
  id: string,
  label: string,
  description: string
): ProviderModelOption {
  return {
    id,
    label,
    description,
    source: "fallback"
  };
}

export const defaultModelOptions: ProviderModelOption[] = [
  createFallbackModel(
    "openrouter/auto",
    "OpenRouter Auto",
    "OpenRouter routes to a broadly available default when you want minimal provider tuning."
  ),
  createFallbackModel(
    "anthropic/claude-3.5-sonnet",
    "Claude Sonnet",
    "Balanced reasoning profile for longer planning loops and post-spawn operator prompts."
  ),
  createFallbackModel(
    "openai/gpt-4.1-mini",
    "GPT-4.1 Mini",
    "Lower-latency generalist model suited to tighter heartbeat budgets and routine CLI tasks."
  ),
  createFallbackModel(
    "google/gemini-2.0-flash",
    "Gemini Flash",
    "Fast multimodal-capable fallback for lightweight inference when OpenRouter catalog loading fails."
  ),
  createFallbackModel(
    "meta-llama/llama-3.3-70b-instruct",
    "Llama 3.3 70B",
    "Open-weight fallback option for stewards who want a capable default before adding Brave search."
  ),
  createFallbackModel(
    "qwen/qwen3.6-plus-preview:free",
    "Qwen 3.6 Plus Preview",
    "Free Qwen preview variant kept pinned in the selector for direct steward choice."
  ),
  createFallbackModel(
    "alibaba/wan-2.6",
    "Alibaba Wan 2.6",
    "Wan 2.6 remains available as a curated fallback entry even when the live catalog is trimmed."
  ),
  createFallbackModel(
    "minimax/minimax-m2.5:free",
    "MiniMax M2.5",
    "Free MiniMax M2.5 option preserved in the curated selector list."
  ),
  createFallbackModel(
    "stepfun/step-3.5-flash:free",
    "Step 3.5 Flash",
    "Free StepFun flash model kept selectable for lightweight, lower-cost inference."
  ),
  createFallbackModel(
    "liquid/lfm-2.5-1.2b-thinking:free",
    "LFM 2.5 1.2B Thinking",
    "Free Liquid thinking model included as a compact fallback choice."
  ),
  createFallbackModel(
    "z-ai/glm-4.5-air:free",
    "GLM 4.5 Air",
    "Free GLM Air variant pinned into the curated fallback set."
  ),
  createFallbackModel(
    "minimax/minimax-m2.7",
    "MiniMax M2.7",
    "MiniMax M2.7 remains explicitly selectable from the curated model list."
  ),
  createFallbackModel(
    "deepseek/deepseek-v3.2",
    "DeepSeek V3.2",
    "DeepSeek V3.2 is kept in the selector for stewards who want that exact model id."
  ),
  createFallbackModel(
    "google/gemini-3-flash-preview",
    "Gemini 3 Flash Preview",
    "Gemini 3 Flash Preview stays available even when live catalog ordering changes."
  ),
  createFallbackModel(
    "x-ai/grok-4.1-fast",
    "Grok 4.1 Fast",
    "Fast Grok variant included as an explicitly selectable curated option."
  )
];
