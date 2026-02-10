#!/usr/bin/env python3
"""Quick script to capture actual MCP context output and test extraction."""
import json
import subprocess
import sys
from pathlib import Path

repo_root = Path(__file__).resolve().parent.parent.parent
binary = str(repo_root / "target" / "debug" / "codegraph")

# Import the extraction function
sys.path.insert(0, str(Path(__file__).resolve().parent))
from eval_mcp import extract_symbols_from_context, normalize

queries_and_expected = [
    ("python-project", "debug why users cannot log in",
     ["login", "verify_password", "get_user_by_email", "User", "hash_password"]),
    ("python-project", "understand how tasks are created",
     ["create_task", "validate_task_title", "get_user_id", "Task", "generate_token"]),
    ("python-project", "add email confirmation to registration",
     ["register", "validate_email", "validate_password", "hash_password", "create_user", "User"]),
    ("typescript-project", "fix the bug where login fails with valid credentials",
     ["login", "verifyPassword", "findUserByEmail", "User", "AuthToken"]),
    ("typescript-project", "understand how orders are created and validated",
     ["createOrder", "validateQuantity", "findProductById", "Order", "OrderItem"]),
    ("typescript-project", "add ability to request a refund for paid orders",
     ["refundPayment", "cancelOrder", "updateOrderStatus", "Order", "PaymentResult"]),
    ("typescript-project", "implement email verification during user registration",
     ["register", "validateEmail", "hashPassword", "createUser", "findUserByEmail", "User"]),
]

for fixture, query, expected in queries_and_expected:
    fixture_path = str(repo_root / "__tests__" / "evaluation" / "fixtures" / fixture)

    proc = subprocess.Popen(
        [binary, "serve", "--mcp", "-p", fixture_path],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    # Initialize
    req1 = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
    proc.stdin.write(req1 + "\n")
    proc.stdin.flush()
    init_resp = proc.stdout.readline()

    # Send notification
    notif = json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
    proc.stdin.write(notif + "\n")
    proc.stdin.flush()

    # Query context
    req2 = json.dumps({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "codegraph_context", "arguments": {"query": query}}
    })
    proc.stdin.write(req2 + "\n")
    proc.stdin.flush()
    resp_line = proc.stdout.readline()

    proc.stdin.close()
    proc.wait(timeout=5)

    resp = json.loads(resp_line)
    content = resp.get("result", {}).get("content", [])
    text = ""
    for block in content:
        if block.get("type") == "text":
            text = block.get("text", "")

    # Extract symbols
    extracted = extract_symbols_from_context(text)
    extracted_norm = {normalize(s) for s in extracted}

    print(f"\n{'='*80}")
    print(f"FIXTURE: {fixture} | QUERY: {query}")
    print(f"{'='*80}")
    print(f"Raw output (first 1500 chars):")
    print(text[:1500])
    print(f"... (total length: {len(text)} chars)")
    print(f"\nExtracted symbols ({len(extracted)}): {sorted(extracted)}")
    print(f"\nExpected: {expected}")

    found = []
    missed = []
    for e in expected:
        if normalize(e) in extracted_norm:
            found.append(e)
        else:
            missed.append(e)

    recall = len(found) / len(expected) if expected else 0
    print(f"Found: {found}")
    print(f"Missed: {missed}")
    print(f"Recall: {recall:.0%}")
