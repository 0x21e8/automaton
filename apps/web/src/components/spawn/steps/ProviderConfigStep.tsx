import type { ProviderModelOption } from "../../../lib/default-models";

interface ProviderConfigStepProps {
  openRouterApiKey: string;
  selectedModelId: string;
  customModelId: string;
  braveSearchApiKey: string;
  modelOptions: ProviderModelOption[];
  isLoadingModels: boolean;
  modelStatusMessage: string;
  onOpenRouterApiKeyChange: (value: string) => void;
  onSelectedModelChange: (value: string) => void;
  onCustomModelChange: (value: string) => void;
  onBraveSearchApiKeyChange: (value: string) => void;
}

function formatPricePerMillion(rawValue: string | null | undefined): string | null {
  if (rawValue === null || rawValue === undefined) {
    return null;
  }

  const parsed = Number(rawValue);

  if (!Number.isFinite(parsed)) {
    return null;
  }

  const perMillion = parsed * 1_000_000;

  if (perMillion === 0) {
    return "$0/M";
  }

  if (perMillion >= 1) {
    return `$${perMillion.toFixed(2)}/M`;
  }

  if (perMillion >= 0.01) {
    return `$${perMillion.toFixed(3)}/M`;
  }

  return `$${perMillion.toFixed(4)}/M`;
}

function formatModelPricing(model: ProviderModelOption): string {
  const promptPrice = formatPricePerMillion(model.pricing?.prompt);
  const completionPrice = formatPricePerMillion(model.pricing?.completion);
  const requestPrice = model.pricing?.request?.trim() ?? null;

  if (promptPrice !== null && completionPrice !== null) {
    return `${promptPrice} in · ${completionPrice} out`;
  }

  if (promptPrice !== null) {
    return `${promptPrice} in`;
  }

  if (completionPrice !== null) {
    return `${completionPrice} out`;
  }

  if (requestPrice !== null && requestPrice !== "") {
    return `$${requestPrice} / request`;
  }

  return "Pricing unavailable";
}

export function ProviderConfigStep({
  openRouterApiKey,
  selectedModelId,
  customModelId,
  braveSearchApiKey,
  modelOptions,
  isLoadingModels,
  modelStatusMessage,
  onOpenRouterApiKeyChange,
  onSelectedModelChange,
  onCustomModelChange,
  onBraveSearchApiKeyChange
}: ProviderConfigStepProps) {
  return (
    <section className="spawn-step">
      <p className="section-label">Step 3</p>
      <h3 className="spawn-step-title">Model &amp; External APIs</h3>
      <p className="spawn-step-copy">
        OpenRouter and Brave are optional. Leave either field blank to keep that
        capability disabled until you configure it later from the steward CLI.
      </p>

      <div className="provider-stack">
        <label className="spawn-field">
          <span className="spawn-field-label">OpenRouter API key</span>
          <input
            className="spawn-input"
            onChange={(event) => {
              onOpenRouterApiKeyChange(event.currentTarget.value);
            }}
            placeholder="sk-or-..."
            type="password"
            value={openRouterApiKey}
          />
        </label>

        <label className="spawn-field">
          <span className="spawn-field-label">Inference model</span>
          <select
            className="spawn-select"
            onChange={(event) => {
              onSelectedModelChange(event.currentTarget.value);
            }}
            value={selectedModelId}
          >
            <option value="">No model selected</option>
            {modelOptions.map((model) => (
              <option key={model.id} value={model.id}>
                {model.label}
                {" · "}
                {formatModelPricing(model)}
                {model.source === "fallback" ? " (fallback)" : ""}
              </option>
            ))}
          </select>
        </label>

        <label className="spawn-field">
          <span className="spawn-field-label">Manual model override</span>
          <input
            className="spawn-input"
            onChange={(event) => {
              onCustomModelChange(event.currentTarget.value);
            }}
            placeholder="anthropic/claude-..."
            type="text"
            value={customModelId}
          />
        </label>

        <p className="spawn-inline-note">
          {isLoadingModels
            ? "Loading live OpenRouter models."
            : modelStatusMessage}
        </p>

        <ul className="provider-model-list">
          {modelOptions.slice(0, 4).map((model) => (
            <li key={model.id}>
              <strong>{model.label}</strong>
              <span>{model.description}</span>
              <span className="provider-model-price">{formatModelPricing(model)}</span>
            </li>
          ))}
        </ul>

        <label className="spawn-field">
          <span className="spawn-field-label">Brave Search API key</span>
          <input
            className="spawn-input"
            onChange={(event) => {
              onBraveSearchApiKeyChange(event.currentTarget.value);
            }}
            placeholder="brv-..."
            type="password"
            value={braveSearchApiKey}
          />
        </label>
      </div>
    </section>
  );
}
