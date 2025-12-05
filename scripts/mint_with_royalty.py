#!/usr/bin/env python3
"""Utility script to call NFTCollection.mintWithRoyalty for local testing.

Usage example:

    python scripts/mint_with_royalty.py \
        --uri https://ipfs.io/ipfs/<CID>/metadata.json \
        --royalty-bps 500

Environment variables (optional, fallback for CLI flags):
    RPC_URL          – HTTP RPC endpoint (e.g. Infura/Alchemy)
    COLLECTION_ADDR  – NFTCollection contract address
    PRIVATE_KEY      – hex string for the signing account

Requirements: `pip install web3 python-dotenv`
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

from dotenv import load_dotenv
from eth_account import Account
from web3 import Web3

ARTIFACT_DEFAULT = Path("artifacts/contracts/v2/NFTCollection.sol/NFTCollection.json")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mint an NFT with royalties via Web3.py")
    parser.add_argument("--uri", required=True, help="Metadata URI (IPFS/HTTPS)")
    parser.add_argument(
        "--royalty-bps",
        type=int,
        default=0,
        help="Royalty in basis points (100 = 1%%, max 1000). Defaults to 0.",
    )
    parser.add_argument(
        "--rpc-url",
        default=os.environ.get("RPC_URL"),
        help="RPC endpoint; falls back to RPC_URL env",
    )
    parser.add_argument(
        "--collection",
        default=os.environ.get("COLLECTION_ADDR"),
        help="NFTCollection address; falls back to COLLECTION_ADDR env",
    )
    parser.add_argument(
        "--private-key",
        default=os.environ.get("PRIVATE_KEY"),
        help="Private key for signing; falls back to PRIVATE_KEY env",
    )
    parser.add_argument(
        "--artifact",
        default=str(ARTIFACT_DEFAULT),
        help="Path to NFTCollection artifact JSON (default: %(default)s)",
    )
    parser.add_argument(
        "--gas-limit",
        type=int,
        default=250_000,
        help="Gas limit for the mint transaction (default: %(default)s)",
    )
    parser.add_argument(
        "--gas-price-gwei",
        type=float,
        default=30.0,
        help="Legacy gas price in gwei (default: %(default)s). Override if network is congested.",
    )
    parser.add_argument(
        "--env-file",
        default=None,
        help="Optional path to .env file to load before reading env vars",
    )
    return parser.parse_args()


def require(value: str | None, name: str) -> str:
    if not value:
        raise SystemExit(f"Missing required value for {name}. Provide --{name.lower()} or set {name} env.")
    return value


def load_artifact(path: str) -> list[dict]:
    artifact_path = Path(path)
    if not artifact_path.exists():
        raise SystemExit(f"Artifact not found: {artifact_path}")
    with artifact_path.open("r", encoding="utf-8") as fh:
        artifact = json.load(fh)
    return artifact["abi"]


def main() -> None:
    args = parse_args()

    if args.env_file:
        load_dotenv(args.env_file)
    else:
        load_dotenv()

    rpc_url = require(args.rpc_url, "RPC_URL")
    collection_addr = Web3.to_checksum_address(require(args.collection, "COLLECTION_ADDR"))
    private_key = require(args.private_key, "PRIVATE_KEY")

    if args.royalty_bps < 0 or args.royalty_bps > 1000:
        raise SystemExit("royalty-bps must be between 0 and 1000 (max 10%)")

    w3 = Web3(Web3.HTTPProvider(rpc_url))
    if not w3.is_connected():
        raise SystemExit("Failed to connect to RPC endpoint")

    abi = load_artifact(args.artifact)
    contract = w3.eth.contract(address=collection_addr, abi=abi)

    signer = Account.from_key(private_key)
    nonce = w3.eth.get_transaction_count(signer.address)

    tx = contract.functions.mintWithRoyalty(args.uri, args.royalty_bps).build_transaction(
        {
            "from": signer.address,
            "nonce": nonce,
            "gas": args.gas_limit,
            "gasPrice": w3.to_wei(args.gas_price_gwei, "gwei"),
        }
    )

    signed = signer.sign_transaction(tx)
    tx_hash = w3.eth.send_raw_transaction(signed.rawTransaction)
    print(f"Submitted tx: {tx_hash.hex()}")

    receipt = w3.eth.wait_for_transaction_receipt(tx_hash)
    status = "success" if receipt.status == 1 else "failed"
    print(f"Receipt status: {status} | block {receipt.blockNumber}")

    try:
        events = contract.events.NFTMinted().process_receipt(receipt)
    except ValueError:
        events = []

    if events:
        evt = events[0]
        token_id = evt.args.tokenId
        print(f"Minted tokenId: {token_id} to {evt.args.to} with royalty {evt.args.royaltyPercentage} bps")
    else:
        print("No NFTMinted event found; check receipt logs manually if needed.")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(1)
