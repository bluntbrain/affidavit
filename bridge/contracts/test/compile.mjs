// Compiles the vault and mocks with solc-js (standard JSON, optimizer on). Run directly to
// print contract sizes; imported by the tests for the ABI and bytecode.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import solc from "solc";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "..", "src");

const files = ["Interfaces.sol", "CausewayVault.sol", "mocks/Mocks.sol"];

export function compile() {
  const sources = Object.fromEntries(files.map((f) => [f, { content: readFileSync(join(src, f), "utf8") }]));
  const input = {
    language: "Solidity",
    sources,
    settings: {
      optimizer: { enabled: true, runs: 200 },
      evmVersion: "cancun",
      outputSelection: { "*": { "*": ["abi", "evm.bytecode.object", "evm.deployedBytecode.object"] } },
    },
  };
  const out = JSON.parse(solc.compile(JSON.stringify(input)));
  const errors = (out.errors ?? []).filter((e) => e.severity === "error");
  if (errors.length) throw new Error(errors.map((e) => e.formattedMessage).join("\n"));
  const warnings = (out.errors ?? []).filter((e) => e.severity === "warning");
  const contracts = {};
  for (const [file, byName] of Object.entries(out.contracts)) {
    for (const [name, c] of Object.entries(byName)) {
      contracts[name] = {
        file,
        abi: c.abi,
        bytecode: "0x" + c.evm.bytecode.object,
        deployedSize: c.evm.deployedBytecode.object.length / 2,
      };
    }
  }
  return { contracts, warnings, solcVersion: solc.version() };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { contracts, warnings, solcVersion } = compile();
  console.log(`solc ${solcVersion}`);
  for (const [name, c] of Object.entries(contracts)) console.log(`${name.padEnd(22)} ${c.deployedSize} bytes deployed`);
  for (const w of warnings) console.log(w.formattedMessage);
}
