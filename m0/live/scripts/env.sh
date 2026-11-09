# Shared paths for the live run.
export LEZ_REPO=<lez-src>
export LEZ_TARGET=<lez-target>
export LIVE=<live>
export SEQ_BIN=$LEZ_TARGET/release/sequencer_service
export WALLET_BIN=$LEZ_TARGET/release/wallet
export DRIVER_BIN=$LEZ_TARGET/release/affidavit_live
export LEE_WALLET_HOME_DIR=$LIVE/wallet_home
export LEE_WALLET_PASSWORD=
export PROGRAM_ELF=/home/user/affidavit/lez/program/target-zkvm/riscv32im-risc0-zkvm-elf/release/affidavit_program
# Funded genesis account of networks/local.json (imported by the Justfile's wallet-import-test-accounts).
export TEST_PRIVATE_KEY=7f273098f25b71e6c005a9519f2678da8d1c7f01f6a27778e2d9948abdf901fb
export BEDROCK_FUNDING_PUBLIC_KEY=2e03b2eff5a45478e7e79668d2a146cf2c5c7925bce927f2b1c67f2ab4fc0d26
