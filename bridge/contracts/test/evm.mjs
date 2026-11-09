// A tiny in-process Ethereum for the tests: ethereumjs VM (Cancun), funded accounts, deploy,
// state-changing sends with gas accounting and decoded custom-error reverts, and view calls.
import { Chain, Common, Hardfork } from "@ethereumjs/common";
import { LegacyTransaction } from "@ethereumjs/tx";
import { Account, Address, bytesToHex, hexToBytes, privateToAddress } from "@ethereumjs/util";
import { VM } from "@ethereumjs/vm";
import { ethers } from "ethers";

export class Evm {
  static async create() {
    const common = new Common({ chain: Chain.Mainnet, hardfork: Hardfork.Cancun });
    const vm = await VM.create({ common });
    return new Evm(vm, common);
  }

  constructor(vm, common) {
    this.vm = vm;
    this.common = common;
    this.nonces = new Map();
  }

  /// A funded EOA from a fixed private key byte.
  async account(byte) {
    const pk = hexToBytes("0x" + byte.toString(16).padStart(2, "0").repeat(32));
    const address = new Address(privateToAddress(pk));
    await this.vm.stateManager.putAccount(address, Account.fromAccountData({ balance: 10n ** 24n, nonce: 0n }));
    this.nonces.set(address.toString(), 0n);
    return { pk, address, hex: address.toString() };
  }

  async #send(from, { to, data, value = 0n, gasLimit = 8_000_000n }) {
    const nonce = this.nonces.get(from.hex) ?? 0n;
    this.nonces.set(from.hex, nonce + 1n);
    const tx = LegacyTransaction.fromTxData(
      { nonce, gasLimit, gasPrice: 1_000_000_000n, to: to ? Address.fromString(to) : undefined, value, data: hexToBytes(data) },
      { common: this.common },
    ).sign(from.pk);
    return this.vm.runTx({ tx });
  }

  /// Deploys a compiled contract; returns a handle with `address`, `iface`, and helpers.
  async deploy(from, compiled, args = [], value = 0n) {
    const iface = new ethers.Interface(compiled.abi);
    const data = compiled.bytecode + iface.encodeDeploy(args).slice(2);
    const res = await this.#send(from, { data, value });
    if (res.execResult.exceptionError) throw new Error(`deploy reverted: ${res.execResult.exceptionError.error}`);
    const address = res.createdAddress.toString();
    return new Contract(this, address, iface);
  }
}

export class Contract {
  constructor(evm, address, iface) {
    this.evm = evm;
    this.address = address;
    this.iface = iface;
  }

  /// State-changing call. Returns `{ gas, logs }` on success; throws `RevertError` with the
  /// decoded custom error name on revert.
  async send(from, fn, args = [], value = 0n) {
    return this.#sendRaw(from, this.iface.encodeFunctionData(fn, args), value);
  }

  async #sendRaw(from, data, value) {
    const nonce = this.evm.nonces.get(from.hex) ?? 0n;
    this.evm.nonces.set(from.hex, nonce + 1n);
    const tx = LegacyTransaction.fromTxData(
      { nonce, gasLimit: 8_000_000n, gasPrice: 1_000_000_000n, to: Address.fromString(this.address), value, data: hexToBytes(data) },
      { common: this.evm.common },
    ).sign(from.pk);
    const res = await this.evm.vm.runTx({ tx });
    if (res.execResult.exceptionError) {
      throw new RevertError(this.iface, res.execResult.exceptionError.error, res.execResult.returnValue);
    }
    const logs = res.execResult.logs.map(([addr, topics, d]) => {
      const log = { address: bytesToHex(addr), topics: topics.map(bytesToHex), data: bytesToHex(d) };
      try {
        return { ...log, parsed: this.iface.parseLog(log) };
      } catch {
        return log;
      }
    });
    return { gas: res.totalGasSpent, logs, receipt: res.receipt };
  }

  /// View call.
  async call(fn, args = []) {
    const data = this.iface.encodeFunctionData(fn, args);
    const r = await this.evm.vm.evm.runCall({
      to: Address.fromString(this.address),
      caller: Address.zero(),
      data: hexToBytes(data),
      gasLimit: 8_000_000n,
    });
    if (r.execResult.exceptionError) throw new RevertError(this.iface, r.execResult.exceptionError.error, r.execResult.returnValue);
    const out = this.iface.decodeFunctionResult(fn, r.execResult.returnValue);
    return out.length === 1 ? out[0] : out;
  }
}

export class RevertError extends Error {
  constructor(iface, kind, returnValue) {
    let name = kind;
    try {
      const parsed = iface.parseError(returnValue);
      if (parsed) name = parsed.name;
    } catch {
      /* not a custom error */
    }
    super(`revert: ${name}`);
    this.kind = kind;
    this.errorName = name;
  }
}
