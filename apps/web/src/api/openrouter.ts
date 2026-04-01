import {
  defaultModelOptions,
  type ProviderModelOption
} from "../lib/default-models";

interface OpenRouterArchitectureRecord {
  input_modalities?: string[];
  output_modalities?: string[];
}

interface OpenRouterPricingRecord {
  completion?: string;
  prompt?: string;
  request?: string;
}

interface OpenRouterModelRecord {
  architecture?: OpenRouterArchitectureRecord;
  id?: string;
  name?: string;
  description?: string;
  pricing?: OpenRouterPricingRecord;
}

interface OpenRouterResponse {
  data?: OpenRouterModelRecord[];
}

const preferredTextModelIds = [
  "minimax/minimax-m2.5:free",
  "stepfun/step-3.5-flash:free",
  "google/gemini-3-flash-preview",
  "deepseek/deepseek-v3.2",
  "qwen/qwen3.6-plus-preview:free",
  "x-ai/grok-4.1-fast",
  "z-ai/glm-4.5-air:free",
  "minimax/minimax-m2.7",
  "alibaba/wan-2.6",
  "liquid/lfm-2.5-1.2b-thinking:free",
  "openrouter/auto",
  "anthropic/claude-3.5-sonnet",
  "openai/gpt-4.1-mini",
  "google/gemini-2.0-flash",
  "meta-llama/llama-3.3-70b-instruct"
] as const;
const MAX_MODEL_OPTIONS = 24;

function supportsTextInputAndOutput(record: OpenRouterModelRecord): boolean {
  const inputModalities = record.architecture?.input_modalities ?? [];
  const outputModalities = record.architecture?.output_modalities ?? [];

  return inputModalities.includes("text") && outputModalities.includes("text");
}

function normalizeModel(record: OpenRouterModelRecord): ProviderModelOption | null {
  if (record.id === undefined || record.id.trim() === "") {
    return null;
  }

  const label =
    record.name?.trim() !== undefined && record.name.trim() !== ""
      ? record.name.trim()
      : record.id;

  return {
    id: record.id,
    label,
    description:
      record.description?.trim() !== undefined && record.description.trim() !== ""
        ? record.description.trim()
        : "Live OpenRouter catalog entry.",
    pricing: {
      completion: record.pricing?.completion?.trim() || null,
      prompt: record.pricing?.prompt?.trim() || null,
      request: record.pricing?.request?.trim() || null
    },
    source: "dynamic"
  };
}

function mergeModelOptions(
  dynamicOptions: ProviderModelOption[],
  fallbackOptions: ProviderModelOption[]
): ProviderModelOption[] {
  const dynamicById = new Map(dynamicOptions.map((option) => [option.id, option] as const));
  const fallbackById = new Map(fallbackOptions.map((option) => [option.id, option] as const));
  const ordered: ProviderModelOption[] = [];
  const seen = new Set<string>();

  for (const modelId of preferredTextModelIds) {
    const option = dynamicById.get(modelId) ?? fallbackById.get(modelId);

    if (option !== undefined && !seen.has(option.id)) {
      ordered.push(option);
      seen.add(option.id);
    }
  }

  for (const option of dynamicOptions) {
    if (!seen.has(option.id)) {
      ordered.push(option);
      seen.add(option.id);
    }

    if (ordered.length >= MAX_MODEL_OPTIONS) {
      return ordered;
    }
  }

  for (const option of fallbackOptions) {
    if (!seen.has(option.id)) {
      ordered.push(option);
      seen.add(option.id);
    }

    if (ordered.length >= MAX_MODEL_OPTIONS) {
      break;
    }
  }

  return ordered;
}

export async function fetchOpenRouterModels(
  signal?: AbortSignal
): Promise<ProviderModelOption[]> {
  const response = await fetch("https://openrouter.ai/api/v1/models?output_modalities=text", {
    headers: {
      Accept: "application/json"
    },
    signal
  });

  if (!response.ok) {
    throw new Error(`OpenRouter catalog request failed with ${response.status}.`);
  }

  const payload = (await response.json()) as OpenRouterResponse;
  const models =
    payload.data
      ?.filter((record) => supportsTextInputAndOutput(record))
      ?.map((record) => normalizeModel(record))
      .filter((record): record is ProviderModelOption => record !== null) ?? [];

  if (models.length === 0) {
    throw new Error("OpenRouter returned an empty model catalog.");
  }

  return mergeModelOptions(models, defaultModelOptions);
}

export { mergeModelOptions };
