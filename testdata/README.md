# Test data

| Path | Contents | Source | License |
|---|---|---|---|
| `mainnet/light-client/bootstrap.json` | `LightClientBootstrap` for block root `0x5afc212a…9275` (slot 7,069,376, sync-committee period 862) | [a16z/helios](https://github.com/a16z/helios) `ethereum/testdata/`, fetched from the mainnet Beacon API | MIT, © a16z |
| `mainnet/light-client/updates.json` | `LightClientUpdate`s of periods 862 to 867 | same | MIT, © a16z |
| `mainnet/light-client/finality.json` | `LightClientFinalityUpdate`, finalised slot 7,109,344 | same | MIT, © a16z |
| `mainnet/light-client/beacon-block-7109344.json` | the full beacon block of the finalised slot (execution block 17,923,026, 167 transactions) | same (`ethereum/testdata/blocks/`) | MIT, © a16z |
| `mainnet/block-21925176/` | `eth_getBlockByNumber`, `eth_getBlockReceipts`, and `eth_getProof` of the beacon deposit contract for block 21,925,176 | Helios `tests/testdata/rpc/` | MIT, © a16z |
| `mainnet/block-26050794/` | `eth_getBlockByHash` and `eth_getBlockReceipts` for block 26,050,794 (BPO2, September 2026) | Helios `tests/testdata/rpc/bpo2/` | MIT, © a16z |
| `spec-tests/{deneb,electra}/` | `light_client/sync` vectors: `light_client_sync`, `advance_finality_without_sync_committee`, `supply_sync_committee_from_past_update` (minimal preset) | [ethereum/consensus-specs](https://github.com/ethereum/consensus-specs) test releases, as vendored by Helios | CC0-1.0 |

The execution blocks have no light-client fixtures that reach them, so tests take their
roots from the block JSON and label it: they exercise inclusion on real tries, while the
binding of roots to finalised headers is exercised on block 17,923,026.

Ethereum chain data is public; the files are reproduced unchanged.
