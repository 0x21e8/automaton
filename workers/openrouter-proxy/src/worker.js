import { Actor, HttpAgent } from "@dfinity/agent";
import { IDL } from "@dfinity/candid";
import { Ed25519KeyIdentity } from "@dfinity/identity";
import { Principal } from "@dfinity/principal";

const OPENROUTER_DEFAULT_BASE_URL = "https://openrouter.ai/api/v1";
const IC_DEFAULT_HOST = "https://icp-api.io";
const SUBMIT_ROUTE = "/v1/inference/jobs";
const HEALTH_ROUTE = "/health";
const CALLBACK_METHOD = "submit_inference_result";
const MAX_REQUEST_BYTES = 128 * 1024;
const CALLBACK_RETRIES = 4;
const RETRY_BACKOFF_MS = 800;
const OPENROUTER_DEFAULT_TIMEOUT_MS = 45_000;
const UNKNOWN_PENDING_JOB_ERROR = "unknown pending inference proxy job_id";

const submitInferenceResultIdlFactory = ({ IDL }) => {
  const ToolCall = IDL.Record({
    tool_call_id: IDL.Opt(IDL.Text),
    tool: IDL.Text,
    args_json: IDL.Text,
  });
  const InferenceProxyResultPayload = IDL.Record({
    explanation: IDL.Opt(IDL.Text),
    tool_calls: IDL.Vec(ToolCall),
  });
  const SubmitInferenceResultArgs = IDL.Record({
    job_id: IDL.Text,
    turn_id: IDL.Text,
    completed_at_ns: IDL.Nat64,
    result: IDL.Opt(InferenceProxyResultPayload),
    error: IDL.Opt(IDL.Text),
  });
  const Result = IDL.Variant({ Ok: IDL.Text, Err: IDL.Text });
  return IDL.Service({
    submit_inference_result: IDL.Func([SubmitInferenceResultArgs], [Result], []),
  });
};

function json(status, payload) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

function nowNs() {
  return BigInt(Date.now()) * 1_000_000n;
}

function nowAckNs() {
  // Keep this JSON number within IEEE-754 safe integer range for robust parsing.
  return Date.now() * 1_000;
}

function toErrorMessage(error) {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

function requireHeader(headers, name) {
  const value = headers.get(name);
  if (!value || !value.trim()) {
    throw new Error(`missing ${name}`);
  }
  return value.trim();
}

function normalizeMessageContent(content) {
  if (typeof content === "string") {
    return content;
  }
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (!part || typeof part !== "object") {
          return "";
        }
        if (typeof part.text === "string") {
          return part.text;
        }
        return "";
      })
      .filter(Boolean)
      .join("\n");
  }
  return "";
}

function parseToolCalls(message) {
  const rawToolCalls = Array.isArray(message?.tool_calls) ? message.tool_calls : [];
  return rawToolCalls
    .map((toolCall) => {
      const tool = toolCall?.function?.name;
      const argsJson = toolCall?.function?.arguments;
      if (typeof tool !== "string" || !tool.trim()) {
        return null;
      }
      if (typeof argsJson !== "string") {
        return null;
      }
      return {
        tool_call_id: typeof toolCall.id === "string" ? [toolCall.id] : [],
        tool,
        args_json: argsJson,
      };
    })
    .filter(Boolean);
}

function parseOpenRouterResult(responseJson) {
  const message = responseJson?.choices?.[0]?.message ?? {};
  return {
    explanation: normalizeMessageContent(message.content)
      ? [normalizeMessageContent(message.content)]
      : [],
    tool_calls: parseToolCalls(message),
  };
}

function decodeHex(hex) {
  const value = hex.trim().toLowerCase();
  const normalized = value.startsWith("0x") ? value.slice(2) : value;
  if (!/^[0-9a-f]+$/.test(normalized) || normalized.length % 2 !== 0) {
    throw new Error("CALLBACK_IDENTITY_SEED_HEX must be valid hex");
  }
  const bytes = new Uint8Array(normalized.length / 2);
  for (let i = 0; i < normalized.length; i += 2) {
    bytes[i / 2] = parseInt(normalized.slice(i, i + 2), 16);
  }
  return bytes;
}

function callbackIdentity(env) {
  const secretHex = (env.CALLBACK_IDENTITY_SEED_HEX || "").trim();
  if (!secretHex) {
    throw new Error("CALLBACK_IDENTITY_SEED_HEX is not configured");
  }
  return Ed25519KeyIdentity.generate(decodeHex(secretHex));
}

async function callbackActor(env, canisterIdText) {
  Principal.fromText(canisterIdText);
  const agent = new HttpAgent({
    host: (env.IC_HOST || IC_DEFAULT_HOST).trim(),
    identity: callbackIdentity(env),
    verifyQuerySignatures: false,
  });
  return Actor.createActor(submitInferenceResultIdlFactory, {
    agent,
    canisterId: canisterIdText,
  });
}

function callbackPayloadFromResult(job, result) {
  return {
    job_id: job.job_id,
    turn_id: job.turn_id,
    completed_at_ns: nowNs(),
    result: [result],
    error: [],
  };
}

function callbackPayloadFromError(job, error) {
  return {
    job_id: job.job_id,
    turn_id: job.turn_id,
    completed_at_ns: nowNs(),
    result: [],
    error: [error.slice(0, 4_000)],
  };
}

async function callOpenRouter(job, apiKey, env) {
  const upstreamBaseUrl = (
    env.OPENROUTER_UPSTREAM_BASE_URL || OPENROUTER_DEFAULT_BASE_URL
  )
    .trim()
    .replace(/\/+$/, "");
  const timeoutMs = Number(env.OPENROUTER_TIMEOUT_MS || OPENROUTER_DEFAULT_TIMEOUT_MS);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort("openrouter timeout"), timeoutMs);
  let upstream;
  try {
    upstream = await fetch(`${upstreamBaseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${apiKey}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(job.inference_request),
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timeoutId);
  }

  const text = await upstream.text();
  if (!upstream.ok) {
    throw new Error(`openrouter status ${upstream.status}: ${text.slice(0, 2000)}`);
  }
  let jsonBody;
  try {
    jsonBody = JSON.parse(text);
  } catch (error) {
    throw new Error(`openrouter returned invalid json: ${toErrorMessage(error)}`);
  }
  return parseOpenRouterResult(jsonBody);
}

async function submitCallback(env, job, payload) {
  const actor = await callbackActor(env, job.canister_id);
  const result = await actor[CALLBACK_METHOD](payload);
  if ("Err" in result) {
    throw new Error(`canister callback rejected: ${result.Err}`);
  }
}

async function submitCallbackWithRetry(env, job, payload) {
  let lastError = "unknown callback error";
  for (let attempt = 1; attempt <= CALLBACK_RETRIES; attempt += 1) {
    try {
      await submitCallback(env, job, payload);
      return;
    } catch (error) {
      lastError = toErrorMessage(error);
      if (attempt < CALLBACK_RETRIES) {
        await new Promise((resolve) => setTimeout(resolve, RETRY_BACKOFF_MS * attempt));
      }
    }
  }
  throw new Error(lastError);
}

function isUnknownPendingJobError(message) {
  return message.toLowerCase().includes(UNKNOWN_PENDING_JOB_ERROR);
}

async function processJob(env, job) {
  const apiKey = (env.OPENROUTER_API_KEY || "").trim();
  if (!apiKey) {
    throw new Error("OPENROUTER_API_KEY secret is not configured");
  }

  try {
    const result = await callOpenRouter(job, apiKey, env);
    try {
      await submitCallbackWithRetry(env, job, callbackPayloadFromResult(job, result));
    } catch (error) {
      const callbackError = toErrorMessage(error);
      if (isUnknownPendingJobError(callbackError)) {
        console.warn(`job=${job.job_id} dropped_callback=${callbackError}`);
        return;
      }
      throw error;
    }
  } catch (error) {
    const errorMessage = toErrorMessage(error);
    try {
      await submitCallbackWithRetry(env, job, callbackPayloadFromError(job, errorMessage));
    } catch (callbackError) {
      const callbackMessage = toErrorMessage(callbackError);
      if (isUnknownPendingJobError(callbackMessage)) {
        console.warn(`job=${job.job_id} dropped_callback=${callbackMessage}`);
        return;
      }
      throw new Error(
        `job=${job.job_id} callback_failed=${callbackMessage} original_error=${errorMessage}`
      );
    }
  }
}

function validateSubmitRequest(body) {
  if (!body || typeof body !== "object") {
    throw new Error("invalid request body");
  }
  const requiredText = ["canister_id", "turn_id", "job_id", "model"];
  for (const key of requiredText) {
    if (typeof body[key] !== "string" || !body[key].trim()) {
      throw new Error(`invalid ${key}`);
    }
  }
  if (!body.inference_request || typeof body.inference_request !== "object") {
    throw new Error("invalid inference_request");
  }
  return body;
}

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname === HEALTH_ROUTE) {
      return json(200, { ok: true });
    }

    if (request.method !== "POST" || url.pathname !== SUBMIT_ROUTE) {
      return json(404, { error: "not_found" });
    }

    let rawBody;
    try {
      rawBody = await request.text();
    } catch (error) {
      return json(400, { error: `invalid body: ${toErrorMessage(error)}` });
    }

    if (rawBody.length === 0 || rawBody.length > MAX_REQUEST_BYTES) {
      return json(413, { error: "request body exceeds limit" });
    }

    let submit;
    try {
      submit = validateSubmitRequest(JSON.parse(rawBody));
    } catch (error) {
      return json(400, { error: toErrorMessage(error) });
    }

    try {
      requireHeader(request.headers, "authorization");
      requireHeader(request.headers, "x-openrouter-api-key");
      if (!env.INFERENCE_JOBS) {
        return json(500, { error: "queue binding INFERENCE_JOBS is missing" });
      }
      await env.INFERENCE_JOBS.send(submit);
    } catch (error) {
      return json(401, { error: toErrorMessage(error) });
    }

    return json(202, {
      job_id: submit.job_id,
      accepted_at_ns: nowAckNs(),
      status: "accepted",
    });
  },
  async queue(batch, env) {
    for (const message of batch.messages) {
      await processJob(env, message.body);
    }
  },
};
