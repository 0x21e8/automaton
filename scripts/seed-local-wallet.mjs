import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const deploymentPath =
  process.env.LOCAL_EVM_DEPLOYMENT_FILE ??
  path.join(rootDir, "tmp", "local-escrow-deployment.json");
const outputPath =
  process.env.LOCAL_EVM_SEED_OUTPUT_FILE ??
  path.join(rootDir, "tmp", "local-wallet-seed.json");

if (!fs.existsSync(deploymentPath)) {
  throw new Error(`missing deployment file: ${deploymentPath}`);
}

const deployment = JSON.parse(fs.readFileSync(deploymentPath, "utf8"));
const usdcTokenAddress = deployment.usdcTokenAddress ?? deployment.mockUsdcAddress;
const walletAddress =
  process.env.LOCAL_EVM_WALLET_ADDRESS ?? "0xCDE2d94d3A757c9d8006258a123D3204E278591b";
const usdcAmount = process.env.LOCAL_EVM_SEED_USDC ?? "250000000";
const ethAmount = process.env.LOCAL_EVM_SEED_ETH ?? "1";
const fundFactorySigner = (process.env.LOCAL_EVM_FUND_FACTORY_SIGNER ?? "1") !== "0";
const factorySignerEthAmount = process.env.LOCAL_EVM_FACTORY_SIGNER_ETH ?? ethAmount;
const factoryCanister = process.env.FACTORY_CANISTER ?? "factory";
const factoryEnvironment = process.env.FACTORY_ENVIRONMENT ?? "local";
const rpcUrl = deployment.rpcUrl;
const fundingAddress = deployment.deployer;
const mintAuthority = deployment.deployer;
const detectedIcpHome = [process.env.ICP_HOME, process.env.LOCAL_ICP_HOME, "/tmp/icp-home"].find(
  (candidate) => candidate && fs.existsSync(candidate)
);

function runCast(args) {
  return execFileSync("cast", args, {
    cwd: rootDir,
    encoding: "utf8"
  }).trim();
}

function calldata(signature, args) {
  return runCast(["calldata", signature, ...args]);
}

function toWei(value) {
  return runCast(["to-wei", value, "ether"]);
}

function runIcp(args) {
  return execFileSync("icp", ["--project-root-override", rootDir, ...args], {
    cwd: rootDir,
    encoding: "utf8",
    env: {
      ...process.env,
      ...(detectedIcpHome ? { ICP_HOME: detectedIcpHome } : {})
    }
  }).trim();
}

function deriveFactorySignerAddress() {
  const output = runIcp([
    "canister",
    "call",
    "-e",
    factoryEnvironment,
    factoryCanister,
    "derive_factory_evm_address",
    "()"
  ]);
  const match = output.match(/0x[a-fA-F0-9]{40}/);
  if (!match) {
    throw new Error(
      `unable to parse factory signer address from derive_factory_evm_address output: ${output}`
    );
  }
  return match[0];
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

async function getBalance(address) {
  const balance = await rpc("eth_getBalance", [address, "latest"]);
  return BigInt(balance);
}

async function getTokenBalance(tokenAddress, address) {
  const balance = await rpc("eth_call", [
    {
      to: tokenAddress,
      data: calldata("balanceOf(address)", [address])
    },
    "latest"
  ]);
  if (balance === "0x") {
    return 0n;
  }

  return BigInt(balance);
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

async function sendTransaction(transaction) {
  const txHash = await rpc("eth_sendTransaction", [transaction]);
  await waitForReceipt(txHash);
  return txHash;
}

const mintTxHash = await sendTransaction({
  from: mintAuthority,
  to: usdcTokenAddress,
  data: calldata("mint(address,uint256)", [walletAddress, usdcAmount])
});

const fundTxHash = await sendTransaction({
  from: fundingAddress,
  to: walletAddress,
  value: `0x${BigInt(toWei(ethAmount)).toString(16)}`
});

let factorySignerSummary = null;
if (fundFactorySigner) {
  const factorySignerAddress = deriveFactorySignerAddress();
  const targetBalanceWei = BigInt(toWei(factorySignerEthAmount));
  const startingBalanceWei = await getBalance(factorySignerAddress);
  let fundSignerTxHash = null;

  if (startingBalanceWei < targetBalanceWei) {
    fundSignerTxHash = await sendTransaction({
      from: fundingAddress,
      to: factorySignerAddress,
      value: `0x${(targetBalanceWei - startingBalanceWei).toString(16)}`
    });
  }

  const endingBalanceWei = await getBalance(factorySignerAddress);
  factorySignerSummary = {
    address: factorySignerAddress,
    targetEthAmount: factorySignerEthAmount,
    startingBalanceWei: startingBalanceWei.toString(),
    endingBalanceWei: endingBalanceWei.toString(),
    fundTxHash: fundSignerTxHash
  };
}

const walletBalances = {
  ethWei: (await getBalance(walletAddress)).toString(),
  usdcRaw: (await getTokenBalance(usdcTokenAddress, walletAddress)).toString()
};

const summary = {
  walletAddress,
  usdcAmount,
  ethAmount,
  mintTxHash,
  fundTxHash,
  balances: walletBalances,
  factorySigner: factorySignerSummary,
  icpHome: detectedIcpHome ?? null,
  network: {
    name: "Base Local Fork",
    rpcUrl,
    chainId: deployment.chainId,
    currencySymbol: "ETH",
    blockExplorerUrl: null
  },
  contracts: {
    usdcTokenAddress,
    mockUsdcAddress: deployment.mockUsdcAddress ?? usdcTokenAddress,
    escrowContractAddress: deployment.escrowContractAddress
  }
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(summary, null, 2)}\n`);
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
