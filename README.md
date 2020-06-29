## Usage

```bash
cargo build --release
RUST_LOG=info ./target/release/ckb-indexer -s /tmp/ckb-indexer-test
```

## RPC

### `get_tip`

Returns the indexed tip block

#### Parameters
    null

#### Returns

    block_hash - indexed tip block hash
    block_number - indexed tip block number

#### Examples

```bash
echo '{
    "id": 2,
    "jsonrpc": "2.0",
    "method": "get_tip"
}' \
| tr -d '\n' \
| curl -H 'content-type: application/json' -d @- \
http://localhost:8116
```

### `get_cells`

Returns the live cells collection by the lock or type script.

#### Parameters

    search_key:
        script - Script
        scrip_type - enum, lock | type
        args_len - maximal prefix search args len, optional
    order: enum, asc | desc
    limit: result size limit
    after_cursor: pagination parameter, optional


#### Returns

    objects - live cells
    last_cursor - pagination parameter

#### Examples

```bash
echo '{
    "id": 2,
    "jsonrpc": "2.0",
    "method": "get_cells",
    "params": [
        {
            "script": {
                "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                "hash_type": "type",
                "args": "0x8211f1b938a107cd53b6302cc752a6fc3965638d"
            },
            "script_type": "lock"
        },
        "asc",
        "0x64"
    ]
}' \
| tr -d '\n' \
| curl -H 'content-type: application/json' -d @- \
http://localhost:8116
```

### `get_transactions`

Returns the transactions collection by the lock or type script.

#### Parameters

    search_key:
        script - Script
        scrip_type - enum, lock | type
        args_len - maximal prefix search args len, optional
    order: enum, asc | desc
    limit: result size limit
    after_cursor: pagination parameter, optional


#### Returns

    objects - transactions
    last_cursor - pagination parameter

#### Examples

```bash
echo '{
    "id": 2,
    "jsonrpc": "2.0",
    "method": "get_transactions",
    "params": [
        {
            "script": {
                "code_hash": "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8",
                "hash_type": "type",
                "args": "0x8211f1b938a107cd53b6302cc752a6fc3965638d"
            },
            "script_type": "lock"
        },
        "asc",
        "0x64"
    ]
}' \
| tr -d '\n' \
| curl -H 'content-type: application/json' -d @- \
http://localhost:8116
```