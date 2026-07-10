import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolveAutomatonComponentRoot } from "./resolve-automaton-component.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const componentRoot = resolveAutomatonComponentRoot();

const deploymentPath =
  process.env.LOCAL_EVM_DEPLOYMENT_FILE ??
  path.join(rootDir, "tmp", "local-escrow-deployment.json");
const outputPath =
  process.env.AUTOMATON_INBOX_DEPLOYMENT_FILE ??
  path.join(rootDir, "tmp", "automaton-inbox-deployment.json");

if (!fs.existsSync(deploymentPath)) {
  throw new Error(`missing local escrow deployment file: ${deploymentPath}`);
}

const deployment = JSON.parse(fs.readFileSync(deploymentPath, "utf8"));
const evmRoot = path.join(componentRoot, "evm");
const rpcUrl = process.env.LOCAL_EVM_RPC_URL ?? deployment.rpcUrl;
const deployer =
  process.env.LOCAL_EVM_DEPLOYER ?? deployment.deployer ?? "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const usdcTokenAddress =
  process.env.AUTOMATON_INBOX_USDC_ADDRESS ??
  deployment.usdcTokenAddress ??
  deployment.mockUsdcAddress;

execFileSync("forge", ["build", "--root", evmRoot], {
  cwd: rootDir,
  stdio: "inherit"
});

const artifactPath = path.join(evmRoot, "out", "Inbox.sol", "Inbox.json");
if (!fs.existsSync(artifactPath)) {
  throw new Error(`missing Inbox artifact: ${artifactPath}`);
}

const artifact = JSON.parse(fs.readFileSync(artifactPath, "utf8"));
const bytecode = artifact?.bytecode?.object;
if (!bytecode || typeof bytecode !== "string") {
  throw new Error(`Inbox artifact at ${artifactPath} has no deployable bytecode`);
}

function abiEncode(signature, args) {
  return execFileSync("cast", ["abi-encode", signature, ...args], {
    cwd: rootDir,
    encoding: "utf8"
  }).trim();
}

async function rpc(method, params) {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now(),
      method,
      params
    })
  });
  const body = await response.json();
  if (body.error) {
    throw new Error(`${method} failed: ${body.error.message}`);
  }
  return body.result;
}

async function waitForReceipt(txHash) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const receipt = await rpc("eth_getTransactionReceipt", [txHash]);
    if (receipt !== null) {
      return receipt;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  throw new Error(`timed out waiting for receipt ${txHash}`);
}

async function deploy(contractBytecode, constructorEncoding = "0x") {
  const txHash = await rpc("eth_sendTransaction", [
    {
      from: deployer,
      data: `${contractBytecode}${constructorEncoding.slice(2)}`
    }
  ]);
  const receipt = await waitForReceipt(txHash);
  if (!receipt.contractAddress) {
    throw new Error(`deployment receipt missing contractAddress for ${txHash}`);
  }
  return { txHash, contractAddress: receipt.contractAddress };
}

const constructorEncoding = abiEncode("constructor(address)", [usdcTokenAddress]);
const { txHash, contractAddress } = await deploy(bytecode, constructorEncoding);
const chainId = Number.parseInt(await rpc("eth_chainId", []), 16);

const summary = {
  rpcUrl,
  chainId,
  deployer,
  sourceRepo: componentRoot,
  usdcTokenAddress,
  inboxContractAddress: contractAddress,
  txHash
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(summary, null, 2)}\n`);
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
