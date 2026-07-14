import type { WalletSession } from "../wallet/useWalletSession";
import {
  bigintToHex,
  encodeErc20ApproveData,
  encodeErc20TransferData,
  stripHexPrefix
} from "./wallet-transaction-helpers";
import { requestJsonRpcResult } from "./spawn-payment";

const MIN_PRICES_FOR_SELECTOR = "0x2bf589a9";
const QUEUE_MESSAGE_SELECTOR = "0xdc0a1b6a";
const QUEUE_MESSAGE_ETH_SELECTOR = "0x9f1b19ac";

function encodeWord(value: bigint): string {
  return value.toString(16).padStart(64, "0");
}

function encodeAddress(address: string): string {
  const value = stripHexPrefix(address.trim());
  if (!/^[0-9a-fA-F]{40}$/.test(value)) throw new Error("A valid EVM address is required.");
  return value.toLowerCase().padStart(64, "0");
}

function encodeString(value: string): string {
  const bytes = new TextEncoder().encode(value);
  const encoded = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${encodeWord(BigInt(bytes.length))}${encoded.padEnd(Math.ceil(encoded.length / 64) * 64, "0")}`;
}

export function encodeMinPricesForData(automaton: string): string {
  return `${MIN_PRICES_FOR_SELECTOR}${encodeAddress(automaton)}`;
}

export function encodeQueueMessageData(
  automaton: string,
  message: string,
  usdcAmount: bigint
): string {
  if (message.trim() === "") throw new Error("Write a message before paying for attention.");
  return `${QUEUE_MESSAGE_SELECTOR}${encodeAddress(automaton)}${encodeWord(96n)}${encodeWord(usdcAmount)}${encodeString(message)}`;
}

export function encodeQueueMessageEthData(automaton: string, message: string): string {
  if (message.trim() === "") throw new Error("Write a message before paying for attention.");
  return `${QUEUE_MESSAGE_ETH_SELECTOR}${encodeAddress(automaton)}${encodeWord(64n)}${encodeString(message)}`;
}

export interface AttentionPrice {
  usdcRaw: bigint;
  ethWei: bigint;
  usesDefault: boolean;
}

export function decodeAttentionPrice(value: string): AttentionPrice {
  const raw = stripHexPrefix(value);
  if (!/^[0-9a-fA-F]{192,}$/.test(raw)) throw new Error("Inbox returned an invalid attention price.");
  return {
    usdcRaw: BigInt(`0x${raw.slice(0, 64)}`),
    ethWei: BigInt(`0x${raw.slice(64, 128)}`),
    usesDefault: BigInt(`0x${raw.slice(128, 192)}`) !== 0n
  };
}

export async function readAttentionPrice(options: {
  automatonAddress: string;
  inboxAddress: string;
  rpcUrl: string;
}): Promise<AttentionPrice> {
  const result = await requestJsonRpcResult<string>(options.rpcUrl, "eth_call", [
    { to: options.inboxAddress, data: encodeMinPricesForData(options.automatonAddress) },
    "latest"
  ]);
  return decodeAttentionPrice(result);
}

export async function sendPaidMessage(options: {
  asset: "usdc" | "eth";
  automatonAddress: string;
  inboxAddress: string;
  message: string;
  price: AttentionPrice;
  usdcAddress: string | null;
  wallet: WalletSession;
  expectedChainId: number;
}): Promise<string[]> {
  if (options.wallet.address === null) throw new Error("Connect a wallet before sending a paid message.");
  if (options.wallet.chainId !== options.expectedChainId) {
    await options.wallet.request({
      method: "wallet_switchEthereumChain",
      params: [{ chainId: `0x${options.expectedChainId.toString(16)}` }]
    });
  }
  if (options.asset === "eth") {
    const hash = await options.wallet.request<string>({
      method: "eth_sendTransaction",
      params: [{
        from: options.wallet.address,
        to: options.inboxAddress,
        value: bigintToHex(options.price.ethWei),
        data: encodeQueueMessageEthData(options.automatonAddress, options.message)
      }]
    });
    return [hash];
  }
  if (options.usdcAddress === null) throw new Error("This being has no indexed USDC contract.");
  const approve = encodeErc20ApproveData(options.inboxAddress, options.price.usdcRaw);
  if (approve === null) throw new Error("Could not encode the USDC approval.");
  const approvalHash = await options.wallet.request<string>({
    method: "eth_sendTransaction",
    params: [{ from: options.wallet.address, to: options.usdcAddress, data: approve }]
  });
  const messageHash = await options.wallet.request<string>({
    method: "eth_sendTransaction",
      params: [{
        from: options.wallet.address,
        to: options.inboxAddress,
        value: bigintToHex(options.price.ethWei),
        data: encodeQueueMessageData(options.automatonAddress, options.message, options.price.usdcRaw)
      }]
  });
  return [approvalHash, messageHash];
}

export async function sendDirectPatronage(options: {
  amountRaw: bigint;
  automatonAddress: string;
  usdcAddress: string;
  wallet: WalletSession;
  expectedChainId: number;
}): Promise<string> {
  if (options.wallet.address === null) throw new Error("Connect a wallet before sending patronage.");
  if (options.wallet.chainId !== options.expectedChainId) {
    await options.wallet.request({ method: "wallet_switchEthereumChain", params: [{ chainId: `0x${options.expectedChainId.toString(16)}` }] });
  }
  if (options.amountRaw <= 0n) throw new Error("Enter a positive patronage amount.");
  const data = encodeErc20TransferData(options.automatonAddress, options.amountRaw);
  if (data === null) throw new Error("Could not encode the patronage transfer.");
  return options.wallet.request<string>({
    method: "eth_sendTransaction",
    params: [{ from: options.wallet.address, to: options.usdcAddress, data }]
  });
}
