# Running Citrea

This document covers how to run Citrea sequencer and a full node locally using a mock DA layer and Bitcoin Regtest.

## Prerequisites
Follow the instructions in [this document.](./dev-setup.md)

## Building and running
Build citrea:
```sh
SKIP_GUEST_BUILD=1 make build
```

### Run on Mock DA
Run on a local da layer, sharable between nodes that run on your computer.

Run sequencer on Mock DA:
```sh
./target/debug/citrea --da-layer mock --rollup-config-path citrea/rollup/configs/mock/sequencer_rollup_config.toml --sequencer-config-path citrea/rollup/configs/mock/sequencer_config.toml --genesis-paths citrea/test-data/genesis/demo-tests/mock
```

Sequencer RPC is accessible at `127.0.0.1:12345`

_Optional_: Run full node on Mock DA:
```sh
./target/debug/citrea --rollup-config-path citrea/rollup/configs/mock/rollup_config.toml --genesis-paths citrea/test-data/genesis/demo-tests/mock 
```

Full node RPC is accessible at `127.0.0.1:12346`

To publish blocks on Mock DA, run these on two seperate terminals:
```sh
 ./citrea/rollup/publish_block.sh

 ./citrea/rollup/publish_da_block.sh
```

### Run on Bitcoin Regtest

Run on local Bitcoin network.

Run Bitcoin Regtest:
```sh
bitcoind -regtest -txindex=1
```
Keep this terminal open.

Create bitcoin wallet for Bitcoin DA adapter.
```sh
bitcoin-cli -regtest createwallet citreatesting
bitcoin-cli -regtest loadwallet citreatesting
```

Mine blocks so that the wallet has BTC:
```sh
bitcoin-cli -regtest -generate 201
```

Edit `bin/citrea/configs/bitcoin-regtest/sequencer_rollup_config.toml` and `bin/citrea/configs/bitcoin-regtest/sequencer_rollup_config.toml` files and put in your rpc url, username and password:

```toml
[da]
# fill here
node_url = ""
# fill here
node_username = ""
# fill here                                       
node_password = ""
```

Run sequencer:
```sh
./target/debug/citrea --da-layer bitcoin --rollup-config-path citrea/rollup/configs/bitcoin-regtest/sequencer_rollup_config.toml --sequencer-config-path citrea/rollup/configs/bitcoin-regtest/sequencer_config.toml --genesis-paths citrea/test-data/genesis/demo-tests/bitcoin-regtest
```

Sequencer RPC is accessible at `127.0.0.1:12345`

_Optional_: Run full node

Run full node:
```sh
./target/debug/citrea --da-layer bitcoin --rollup-config-path citrea/rollup/configs/bitcoin-regtest/rollup_config.toml --genesis-paths citrea/test-data/genesis/demo-tests/bitcoin-regtest
```

Full node RPC is accessible at `127.0.0.1:12346`

To publish blocks on Bitcoin Regtest, run this and keep the terminal open:
```sh
 ./citrea/rollup/publish_block.sh
```

To delete sequencer or full nodes databases run:
```sh
make clean-node
```

## Testing

To run tests:
```sh
SKIP_GUEST_BUILD=1 make test
```
This will run [`cargo nextest`](https://nexte.st).