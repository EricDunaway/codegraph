#!/usr/bin/env python3
"""
Evaluation harness for CodeGraph Rust MCP server.

Spawns the MCP server over stdio, sends JSON-RPC tool calls,
and compares results against the same ground truth used by the
TypeScript evaluation suite.

Usage:
    python __tests__/evaluation/eval_mcp.py [--binary TARGET_DIR/codegraph]
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Ground truth (translated from the TypeScript fixtures)
# ---------------------------------------------------------------------------

@dataclass
class TestCase:
    id: str
    description: str
    query: str
    type: str  # search | context | callers | callees | impact
    expected_symbols: list[str]
    irrelevant_symbols: list[str]
    target_symbol: Optional[str] = None
    min_recall: float = 0.0
    min_precision: float = 0.0


@dataclass
class Fixture:
    name: str
    path: str  # relative to repo root
    language: str
    total_files: int
    test_cases: list[TestCase]


PYTHON_FIXTURE = Fixture(
    name="python-taskmanager",
    path="__tests__/evaluation/fixtures/python-project",
    language="python",
    total_files=5,
    test_cases=[
        # Search tests
        TestCase(
            id="py-search-auth",
            description="Search for authentication functionality",
            query="authentication login",
            type="search",
            expected_symbols=["AuthService", "login", "register", "verify_password"],
            irrelevant_symbols=["TaskService", "validate_task_title", "Project"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        TestCase(
            id="py-search-task",
            description="Search for task management",
            query="task create complete",
            type="search",
            expected_symbols=["TaskService", "create_task", "complete_task", "Task"],
            irrelevant_symbols=["AuthService", "validate_email", "hash_password"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        TestCase(
            id="py-search-validation",
            description="Search for validation",
            query="validate",
            type="search",
            expected_symbols=["validate_email", "validate_password", "validate_task_title"],
            irrelevant_symbols=["hash_password", "generate_token", "TaskService"],
            min_recall=0.8,
            min_precision=0.6,
        ),
        # Context tests
        TestCase(
            id="py-context-login-bug",
            description="Build context for fixing login issues",
            query="debug why users cannot log in",
            type="context",
            expected_symbols=["login", "verify_password", "get_user_by_email", "User", "hash_password"],
            irrelevant_symbols=["TaskService", "validate_task_title", "Project", "Task"],
            min_recall=0.8,
            min_precision=0.6,
        ),
        TestCase(
            id="py-context-task-creation",
            description="Build context for task creation flow",
            query="understand how tasks are created",
            type="context",
            expected_symbols=["create_task", "validate_task_title", "get_user_id", "Task", "generate_token"],
            irrelevant_symbols=["validate_email", "hash_password", "register", "Project"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        TestCase(
            id="py-context-user-registration",
            description="Build context for user registration",
            query="add email confirmation to registration",
            type="context",
            expected_symbols=["register", "validate_email", "validate_password", "hash_password", "create_user", "User"],
            irrelevant_symbols=["TaskService", "validate_task_title", "Task", "Project"],
            min_recall=0.7,
            min_precision=0.6,
        ),
        # Callers tests
        TestCase(
            id="py-callers-get_user_id",
            description="Find all callers of get_user_id",
            query="get_user_id",
            type="callers",
            target_symbol="get_user_id",
            expected_symbols=["create_task", "get_task", "get_user_tasks"],
            irrelevant_symbols=["login", "validate_email", "hash_password"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="py-callers-validate_email",
            description="Find all callers of validate_email",
            query="validate_email",
            type="callers",
            target_symbol="validate_email",
            expected_symbols=["register"],
            irrelevant_symbols=["TaskService", "validate_password", "hash_password"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="py-callers-generate_token",
            description="Find all callers of generate_token",
            query="generate_token",
            type="callers",
            target_symbol="generate_token",
            expected_symbols=["register", "login", "create_task"],
            irrelevant_symbols=["validate_email", "validate_password"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        # Callees tests
        TestCase(
            id="py-callees-login",
            description="Find what login calls",
            query="login",
            type="callees",
            target_symbol="login",
            expected_symbols=["get_user_by_email", "verify_password", "generate_token"],
            irrelevant_symbols=["validate_email", "hash_password", "validate_task_title"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="py-callees-create_task",
            description="Find what create_task calls",
            query="create_task",
            type="callees",
            target_symbol="create_task",
            expected_symbols=["get_user_id", "validate_task_title", "generate_token", "create_task"],
            irrelevant_symbols=["validate_email", "hash_password"],
            min_recall=0.8,
            min_precision=0.8,
        ),
        # Impact tests
        TestCase(
            id="py-impact-generate_token",
            description="Impact of changing generate_token",
            query="generate_token",
            type="impact",
            target_symbol="generate_token",
            expected_symbols=["register", "login", "create_task"],
            irrelevant_symbols=["validate_email", "validate_task_title"],
            min_recall=0.8,
            min_precision=0.7,
        ),
        TestCase(
            id="py-impact-get_user_id",
            description="Impact of changing get_user_id",
            query="get_user_id",
            type="impact",
            target_symbol="get_user_id",
            expected_symbols=["create_task", "get_task", "get_user_tasks", "complete_task", "delete_task"],
            irrelevant_symbols=["register", "validate_email", "hash_password"],
            min_recall=0.8,
            min_precision=0.7,
        ),
    ],
)

TYPESCRIPT_FIXTURE = Fixture(
    name="typescript-ecommerce",
    path="__tests__/evaluation/fixtures/typescript-project",
    language="typescript",
    total_files=9,
    test_cases=[
        # Search tests
        TestCase(
            id="ts-search-login",
            description="Search for login functionality",
            query="login",
            type="search",
            expected_symbols=["login", "AuthService"],
            irrelevant_symbols=["PaymentService", "OrderService", "calculateTotal"],
            min_recall=0.8,
            min_precision=0.5,
        ),
        TestCase(
            id="ts-search-validation",
            description="Search for validation functions",
            query="validate",
            type="search",
            expected_symbols=["validateEmail", "validatePassword", "validateQuantity", "validatePrice", "validateToken"],
            irrelevant_symbols=["hashPassword", "generateToken", "calculateTotal"],
            min_recall=0.6,
            min_precision=0.6,
        ),
        TestCase(
            id="ts-search-payment",
            description="Search for payment processing",
            query="payment process",
            type="search",
            expected_symbols=["PaymentService", "processPayment", "payOrder"],
            irrelevant_symbols=["AuthService", "UserService", "validateEmail"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        # Context tests
        TestCase(
            id="ts-context-login-bug",
            description="Build context for fixing a login bug",
            query="fix the bug where login fails with valid credentials",
            type="context",
            expected_symbols=["login", "verifyPassword", "findUserByEmail", "User", "AuthToken"],
            irrelevant_symbols=["OrderService", "PaymentService", "calculateTotal", "validateQuantity", "Product"],
            min_recall=0.8,
            min_precision=0.6,
        ),
        TestCase(
            id="ts-context-order-creation",
            description="Build context for order creation flow",
            query="understand how orders are created and validated",
            type="context",
            expected_symbols=["createOrder", "validateQuantity", "findProductById", "Order", "OrderItem"],
            irrelevant_symbols=["register", "validateEmail", "hashPassword", "UserService"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        TestCase(
            id="ts-context-add-refund",
            description="Build context for adding refund functionality",
            query="add ability to request a refund for paid orders",
            type="context",
            expected_symbols=["refundPayment", "cancelOrder", "updateOrderStatus", "Order", "PaymentResult"],
            irrelevant_symbols=["register", "validateEmail", "hashPassword"],
            min_recall=0.7,
            min_precision=0.5,
        ),
        TestCase(
            id="ts-context-user-registration",
            description="Build context for user registration",
            query="implement email verification during user registration",
            type="context",
            expected_symbols=["register", "validateEmail", "hashPassword", "createUser", "findUserByEmail", "User"],
            irrelevant_symbols=["OrderService", "PaymentService", "calculateTotal", "Product"],
            min_recall=0.7,
            min_precision=0.6,
        ),
        # Callers tests
        TestCase(
            id="ts-callers-validateEmail",
            description="Find all callers of validateEmail",
            query="validateEmail",
            type="callers",
            target_symbol="validateEmail",
            expected_symbols=["register", "updateProfile"],
            irrelevant_symbols=["OrderService", "PaymentService", "validateQuantity"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="ts-callers-findUserByEmail",
            description="Find all callers of findUserByEmail",
            query="findUserByEmail",
            type="callers",
            target_symbol="findUserByEmail",
            expected_symbols=["register", "login", "getUserByEmail", "updateProfile"],
            irrelevant_symbols=["OrderService", "PaymentService", "findProductById"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="ts-callers-generateToken",
            description="Find all callers of generateToken",
            query="generateToken",
            type="callers",
            target_symbol="generateToken",
            expected_symbols=["register", "createToken", "processPayment", "refundPayment"],
            irrelevant_symbols=["validateEmail", "validateQuantity", "calculateTotal"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        # Callees tests
        TestCase(
            id="ts-callees-login",
            description="Find what login calls",
            query="login",
            type="callees",
            target_symbol="login",
            expected_symbols=["findUserByEmail", "verifyPassword", "createToken"],
            irrelevant_symbols=["hashPassword", "validateQuantity", "calculateTotal"],
            min_recall=1.0,
            min_precision=1.0,
        ),
        TestCase(
            id="ts-callees-createOrder",
            description="Find what createOrder calls",
            query="createOrder",
            type="callees",
            target_symbol="createOrder",
            expected_symbols=["validateToken", "validateQuantity", "findProductById", "calculateTotal", "generateOrderId", "createOrder", "updateProductStock"],
            irrelevant_symbols=["validateEmail", "hashPassword", "refundPayment"],
            min_recall=0.8,
            min_precision=0.8,
        ),
        # Impact tests
        TestCase(
            id="ts-impact-generateToken",
            description="Impact of changing generateToken",
            query="generateToken",
            type="impact",
            target_symbol="generateToken",
            expected_symbols=["register", "createToken", "processPayment", "refundPayment", "login", "refreshToken", "payOrder", "cancelOrder"],
            irrelevant_symbols=["validateQuantity", "validatePrice"],
            min_recall=0.7,
            min_precision=0.6,
        ),
        TestCase(
            id="ts-impact-validateToken",
            description="Impact of changing validateToken",
            query="validateToken",
            type="impact",
            target_symbol="validateToken",
            expected_symbols=["refreshToken", "createOrder", "getOrder", "getUserOrders", "payOrder", "cancelOrder"],
            irrelevant_symbols=["validateEmail", "validateQuantity", "hashPassword"],
            min_recall=0.8,
            min_precision=0.7,
        ),
    ],
)


# ---------------------------------------------------------------------------
# MCP JSON-RPC client over stdio
# ---------------------------------------------------------------------------

class McpClient:
    """Thin JSON-RPC client that talks to `codegraph serve --mcp` over stdio."""

    def __init__(self, binary: str, project_path: str):
        self.binary = binary
        self.project_path = project_path
        self.proc: Optional[subprocess.Popen] = None
        self._id = 0

    def start(self):
        self.proc = subprocess.Popen(
            [self.binary, "serve", "--mcp", "-p", self.project_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        # Send initialize
        resp = self._call("initialize", {})
        if resp is None or "error" in resp:
            raise RuntimeError(f"MCP initialize failed: {resp}")
        # Send initialized notification (no id = notification, no response expected)
        self._send_notification("notifications/initialized", {})

    def stop(self):
        if self.proc:
            self._call("shutdown", {})
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
            self.proc = None

    def call_tool(self, tool_name: str, arguments: dict) -> dict:
        """Call an MCP tool and return the parsed response."""
        resp = self._call("tools/call", {"name": tool_name, "arguments": arguments})
        return resp

    def _next_id(self) -> int:
        self._id += 1
        return self._id

    def _call(self, method: str, params: dict) -> dict:
        req = {
            "jsonrpc": "2.0",
            "id": self._next_id(),
            "method": method,
            "params": params,
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()

        line = self.proc.stdout.readline()
        if not line:
            stderr = self.proc.stderr.read()
            raise RuntimeError(f"MCP server closed stdout. stderr: {stderr}")
        return json.loads(line)

    def _send_notification(self, method: str, params: dict):
        req = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()


# ---------------------------------------------------------------------------
# Response parsing
# ---------------------------------------------------------------------------

# Matches lines like: - function `qualified_name` (file:line) [id: hash]
SYMBOL_LINE_RE = re.compile(r"- \w+ `([^`]+)`")


def extract_symbols_from_text(text: str) -> list[str]:
    """Extract symbol names from MCP tool text output."""
    symbols = []
    for match in SYMBOL_LINE_RE.finditer(text):
        qualified = match.group(1)
        # qualified_name is like "auth.py::login" or "validation.py::validate_email"
        # Extract the simple name (last component after ::)
        simple = qualified.split("::")[-1] if "::" in qualified else qualified
        # Also handle dots like "AuthService.login"
        symbols.append(simple)
    return symbols


def extract_symbols_from_context(text: str) -> list[str]:
    """Extract symbol names from context markdown output.

    The context tool returns markdown in this format:

        # Context: <query>
        **N symbols** across **M files**

        ## Files
        - `auth.py` (3 symbols)

        ## Symbols
        ### function `auth.py::login`
        - **File:** `auth.py:57:68`
        - **Signature:** `def login(self, email, password)`
        > Docstring text
        ```python
        def login(self, email: str, password: str) -> Optional[str]:
            user = db.get_user_by_email(email)
            ...
        ```

        ## Relationships
        - `hash1` --[calls]--> `hash2`

    We extract symbol names from all of these sections.
    """
    symbols = set()

    # 1. Symbol section headers: "### kind `qualified_name`"
    #    e.g. "### function `auth.py::login`" or "### class `models.py::User`"
    for m in re.finditer(r"###\s+\w+\s+`([^`]+)`", text):
        qualified = m.group(1)
        # Extract simple name from "file.py::ClassName.method" or "file.py::func"
        after_file = qualified.split("::")[-1] if "::" in qualified else qualified
        # Handle dotted names like "AuthService.login" -> add both parts and compound
        parts = after_file.split(".")
        for part in parts:
            if part and not _is_file_name(part):
                symbols.add(part)

    # 2. Signature lines: "- **Signature:** `def login(self, email, password)`"
    #    or "- **Signature:** `async createOrder(token: string, items: OrderItem[]): Promise<Order>`"
    for m in re.finditer(r"\*\*Signature:\*\*\s*`([^`]+)`", text):
        sig = m.group(1)
        _extract_identifiers_from_signature(sig, symbols)

    # 3. Code blocks: extract function/class/type definitions and call sites
    in_code_block = False
    code_lang = ""
    for line in text.split("\n"):
        if line.startswith("```") and not in_code_block:
            in_code_block = True
            code_lang = line[3:].strip()
            continue
        elif line.startswith("```") and in_code_block:
            in_code_block = False
            code_lang = ""
            continue

        if in_code_block:
            _extract_identifiers_from_code(line, code_lang, symbols)

    # 4. Relationship lines: "- `hash1` --[calls]--> `hash2`"
    #    These are node IDs (hashes), not useful for symbol matching.
    #    Skip them.

    # 5. Backtick-quoted identifiers outside of file references
    #    e.g. `validate_email`, `User`, `AuthService`
    #    But skip file paths like `auth.py:57:68` and `auth.py`
    for m in re.finditer(r"`([a-zA-Z_]\w*(?:\.\w+)*)`", text):
        name = m.group(1)
        # Skip file names (contain dots with common extensions)
        if _is_file_name(name):
            continue
        # Handle dotted names
        parts = name.split(".")
        for part in parts:
            if part and re.match(r"^[a-zA-Z_]\w*$", part) and not _is_file_name(part):
                symbols.add(part)

    # 6. File section entries: "- `auth.py` (3 symbols)" -- skip these
    #    Already handled by the file name filter above.

    # Remove common noise words that aren't actual symbols
    noise = {
        "self", "None", "True", "False", "str", "int", "float", "bool",
        "list", "dict", "set", "tuple", "Optional", "List", "Dict", "Tuple",
        "return", "if", "else", "for", "while", "class", "def", "import",
        "from", "async", "await", "const", "let", "var", "function",
        "export", "interface", "type", "enum", "string", "number", "boolean",
        "void", "null", "undefined", "Promise", "any", "new", "this",
        "not", "and", "or", "in", "is", "with", "as", "try", "except",
        "raise", "throw", "Error", "Date",
    }
    symbols -= noise

    return list(symbols)


def _is_file_name(name: str) -> bool:
    """Check if a name looks like a file path rather than a symbol."""
    file_extensions = {
        ".py", ".ts", ".tsx", ".js", ".jsx", ".rs", ".go", ".java",
        ".rb", ".php", ".swift", ".kt", ".c", ".h", ".cpp", ".cs",
    }
    for ext in file_extensions:
        if name.endswith(ext):
            return True
    # Also match file:line patterns
    if re.match(r".*\.\w+:\d+", name):
        return True
    return False


def _extract_identifiers_from_signature(sig: str, symbols: set):
    """Extract meaningful identifiers from a function/method signature.

    Examples:
        def login(self, email: str, password: str) -> Optional[str]
        async createOrder(token: string, items: OrderItem[]): Promise<Order>
        fn get_user_by_email(&self, email: &str) -> Option<User>
    """
    # Extract the function/method name
    # Python: def name(  or  async def name(
    m = re.search(r"(?:async\s+)?(?:def|fn)\s+(\w+)", sig)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS: async? name( or name(
    m = re.search(r"(?:async\s+)?(\w+)\s*\(", sig)
    if m and m.group(1) not in ("def", "fn", "function", "async"):
        symbols.add(m.group(1))

    # Extract type names from type annotations
    # Match capitalized identifiers that look like type/class names
    for m in re.finditer(r"\b([A-Z]\w+)\b", sig):
        name = m.group(1)
        if name not in ("Optional", "List", "Dict", "Tuple", "Set",
                        "Promise", "Partial", "Record", "Array",
                        "String", "Number", "Boolean", "None", "True", "False"):
            symbols.add(name)


def _extract_identifiers_from_code(line: str, lang: str, symbols: set):
    """Extract symbol names from a line of source code.

    Looks for function/method definitions, class definitions,
    type definitions, function calls, and import statements.
    """
    stripped = line.strip()
    if not stripped or stripped.startswith("#") or stripped.startswith("//"):
        return

    # Python function/method definitions
    m = re.match(r"\s*(?:async\s+)?def\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # Python class definitions
    m = re.match(r"\s*class\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # Python import statements
    # from module import name1, name2
    m = re.match(r"\s*from\s+\w+\s+import\s+(.+)", line)
    if m:
        for name in re.findall(r"(\w+)", m.group(1)):
            if name not in ("as", "import"):
                symbols.add(name)

    # import module
    m = re.match(r"\s*import\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS function definitions
    m = re.match(r"\s*(?:export\s+)?(?:async\s+)?function\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS class definitions
    m = re.match(r"\s*(?:export\s+)?class\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS interface definitions
    m = re.match(r"\s*(?:export\s+)?interface\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS type alias definitions
    m = re.match(r"\s*(?:export\s+)?type\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # TypeScript/JS method definitions in classes
    m = re.match(r"\s*(?:async\s+)?(\w+)\s*\([^)]*\)\s*(?::\s*\S+)?\s*\{", line)
    if m and m.group(1) not in ("if", "for", "while", "switch", "catch", "function"):
        symbols.add(m.group(1))

    # TypeScript/JS import statements
    # import { Name1, Name2 } from './module'
    m = re.match(r"\s*import\s+\{([^}]+)\}", line)
    if m:
        for name in re.findall(r"(\w+)", m.group(1)):
            if name not in ("as", "from", "type"):
                symbols.add(name)

    # Function calls: name( or obj.name(
    for m in re.finditer(r"(?:^|[\s=({,])(\w+)\s*\(", line):
        name = m.group(1)
        if name not in ("if", "for", "while", "switch", "catch", "return",
                        "print", "len", "str", "int", "float", "bool",
                        "list", "dict", "set", "tuple", "type",
                        "isinstance", "hasattr", "getattr", "super",
                        "format", "range", "enumerate", "zip", "map", "filter",
                        "def", "class", "function", "async", "await", "not"):
            symbols.add(name)

    # Method calls: obj.method(
    for m in re.finditer(r"\.(\w+)\s*\(", line):
        name = m.group(1)
        if name not in ("get", "set", "has", "add", "remove", "delete",
                        "append", "extend", "pop", "push", "join", "split",
                        "strip", "replace", "format", "encode", "decode",
                        "keys", "values", "items", "update", "copy",
                        "startswith", "endswith", "lower", "upper",
                        "then", "catch", "finally", "map", "filter",
                        "reduce", "forEach", "find", "some", "every",
                        "log", "error", "warn", "info", "hexdigest"):
            symbols.add(name)

    # Attribute access for known patterns: obj.attribute (for type-like names)
    for m in re.finditer(r"\.(\w+)", line):
        name = m.group(1)
        # Only add if it starts with uppercase (likely a class/type reference)
        if name and name[0].isupper():
            symbols.add(name)

    # Rust function/method definitions
    m = re.match(r"\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # Rust struct/enum/trait definitions
    m = re.match(r"\s*(?:pub\s+)?(?:struct|enum|trait|impl)\s+(\w+)", line)
    if m:
        symbols.add(m.group(1))

    # Capitalized identifiers in the line (likely class/type names)
    for m in re.finditer(r"\b([A-Z][a-zA-Z0-9_]*)\b", line):
        name = m.group(1)
        # Filter out common non-symbol capitalized words
        if name not in ("None", "True", "False", "Optional", "List", "Dict",
                        "Tuple", "Set", "Union", "Any", "Type",
                        "Promise", "Partial", "Record", "Array", "Map",
                        "String", "Number", "Boolean", "Symbol", "BigInt",
                        "Error", "Date", "RegExp", "Math", "JSON",
                        "Object", "Function", "Infinity", "NaN",
                        "GET", "POST", "PUT", "DELETE", "PATCH",
                        "OK", "NULL", "EOF", "TODO", "FIXME",
                        "If", "For", "While", "Return", "Import", "From",
                        "Cannot", "Invalid", "Insufficient"):
            symbols.add(name)


def get_tool_text(response: dict) -> str:
    """Extract the text content from an MCP tool response."""
    result = response.get("result", {})
    if isinstance(result, dict):
        content = result.get("content", [])
        for block in content:
            if block.get("type") == "text":
                return block.get("text", "")
    return ""


# ---------------------------------------------------------------------------
# Symbol matching (mirrors the TS evaluation logic)
# ---------------------------------------------------------------------------

def normalize(symbol: str) -> str:
    """Normalize a symbol name for comparison."""
    # Take the last component (after . or ::)
    parts = symbol.replace("::", ".").split(".")
    return parts[-1].lower()


def symbol_matches(symbol: str, candidates: set[str]) -> bool:
    """Check if symbol matches any candidate (case-insensitive, last-component match)."""
    norm = normalize(symbol)
    return norm in candidates


# ---------------------------------------------------------------------------
# Test runner
# ---------------------------------------------------------------------------

@dataclass
class TestResult:
    test_id: str
    test_type: str
    passed: bool
    precision: float
    recall: float
    f1: float
    true_positives: list[str]
    false_positives: list[str]
    false_negatives: list[str]
    error: Optional[str] = None


def run_test_case(client: McpClient, tc: TestCase) -> TestResult:
    """Run a single test case against the MCP server."""
    retrieved_symbols: list[str] = []
    error = None

    try:
        if tc.type == "search":
            resp = client.call_tool("codegraph_search", {"query": tc.query, "limit": 20})
            text = get_tool_text(resp)
            retrieved_symbols = extract_symbols_from_text(text)

        elif tc.type == "context":
            resp = client.call_tool("codegraph_context", {"query": tc.query, "max_tokens": 8000})
            text = get_tool_text(resp)
            retrieved_symbols = extract_symbols_from_context(text)

        elif tc.type in ("callers", "callees", "impact"):
            # First search for the target symbol to get its node_id
            target = tc.target_symbol or tc.query
            search_resp = client.call_tool("codegraph_search", {"query": target, "limit": 5})
            search_text = get_tool_text(search_resp)

            # Extract node_id from search results
            id_match = re.search(r"\[id: ([^\]]+)\]", search_text)
            if not id_match:
                error = f"Could not find node_id for '{target}'"
            else:
                node_id = id_match.group(1)

                if tc.type == "callers":
                    resp = client.call_tool("codegraph_callers", {"node_id": node_id})
                elif tc.type == "callees":
                    resp = client.call_tool("codegraph_callees", {"node_id": node_id})
                else:  # impact
                    resp = client.call_tool("codegraph_impact", {"node_id": node_id, "max_depth": 3})

                text = get_tool_text(resp)
                retrieved_symbols = extract_symbols_from_text(text)

    except Exception as e:
        error = str(e)

    # Normalize expected/irrelevant into sets of lowercase last-components
    expected_set = {normalize(s) for s in tc.expected_symbols}
    irrelevant_set = {normalize(s) for s in tc.irrelevant_symbols}

    # Compute metrics
    true_positives = []
    false_positives = []
    seen = set()

    for sym in retrieved_symbols:
        norm = normalize(sym)
        if norm in seen:
            continue
        seen.add(norm)

        if norm in expected_set:
            true_positives.append(sym)
        elif norm in irrelevant_set:
            false_positives.append(sym)

    false_negatives = [s for s in tc.expected_symbols if normalize(s) not in {normalize(tp) for tp in true_positives}]

    total_retrieved = len(true_positives) + len(false_positives)
    precision = len(true_positives) / total_retrieved if total_retrieved > 0 else 0.0
    recall = len(true_positives) / len(tc.expected_symbols) if tc.expected_symbols else 0.0
    f1 = 2 * precision * recall / (precision + recall) if (precision + recall) > 0 else 0.0

    # Pass with 20% margin (same as TS tests)
    passed_recall = tc.min_recall == 0 or recall >= tc.min_recall * 0.8
    passed_precision = tc.min_precision == 0 or precision >= tc.min_precision * 0.8
    passed = passed_recall and passed_precision and error is None

    return TestResult(
        test_id=tc.id,
        test_type=tc.type,
        passed=passed,
        precision=precision,
        recall=recall,
        f1=f1,
        true_positives=true_positives,
        false_positives=false_positives,
        false_negatives=false_negatives,
        error=error,
    )


# ---------------------------------------------------------------------------
# Index fixtures via CLI
# ---------------------------------------------------------------------------

def index_fixture(binary: str, fixture_path: str):
    """Initialize and index a fixture project."""
    codegraph_dir = os.path.join(fixture_path, ".codegraph")
    if os.path.exists(codegraph_dir):
        shutil.rmtree(codegraph_dir)

    subprocess.run([binary, "init", fixture_path], check=True, capture_output=True, text=True)
    result = subprocess.run([binary, "index", fixture_path], check=True, capture_output=True, text=True)
    print(f"  {result.stderr.strip() or result.stdout.strip()}")


# ---------------------------------------------------------------------------
# Pretty printing
# ---------------------------------------------------------------------------

def print_results_table(results: list[TestResult], fixture_name: str):
    print(f"\n{'=' * 88}")
    print(f"  {fixture_name} Results")
    print("=" * 88)
    print()
    print(f"  {'Test ID':<38} {'Type':<10} {'Prec':>5}  {'Recall':>6}  {'F1':>5}  {'Status'}")
    print(f"  {'-' * 82}")

    for r in results:
        status = "PASS" if r.passed else "FAIL"
        err = f" ({r.error})" if r.error else ""
        prec = f"{r.precision * 100:.0f}%"
        recall = f"{r.recall * 100:.0f}%"
        f1 = f"{r.f1 * 100:.0f}%"
        print(f"  {r.test_id:<38} {r.test_type:<10} {prec:>5}  {recall:>6}  {f1:>5}  {status}{err}")

        if r.false_negatives and not r.passed:
            print(f"    missed: {', '.join(r.false_negatives)}")

    # Averages
    avg_p = sum(r.precision for r in results) / len(results)
    avg_r = sum(r.recall for r in results) / len(results)
    avg_f1 = sum(r.f1 for r in results) / len(results)
    pass_rate = sum(1 for r in results if r.passed) / len(results)

    print(f"  {'-' * 82}")
    print(f"  {'AVERAGE':<38} {'':10} {avg_p * 100:4.0f}%  {avg_r * 100:5.0f}%  {avg_f1 * 100:4.0f}%  {pass_rate * 100:.0f}% pass")
    print()


def print_summary(all_results: dict[str, list[TestResult]]):
    print("\n" + "=" * 88)
    print("  OVERALL SUMMARY")
    print("=" * 88)

    total_tests = 0
    total_passed = 0
    for fixture_name, results in all_results.items():
        passed = sum(1 for r in results if r.passed)
        total = len(results)
        total_tests += total
        total_passed += passed

        # Break down by type
        by_type: dict[str, list[TestResult]] = {}
        for r in results:
            by_type.setdefault(r.test_type, []).append(r)

        print(f"\n  {fixture_name}: {passed}/{total} passed")
        for ttype, tres in sorted(by_type.items()):
            tp = sum(1 for r in tres if r.passed)
            print(f"    {ttype:<10}: {tp}/{len(tres)}")

    print(f"\n  Total: {total_passed}/{total_tests} passed")
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="CodeGraph MCP Evaluation Harness")
    parser.add_argument(
        "--binary",
        default=None,
        help="Path to codegraph binary (default: cargo build and use target/debug/codegraph)",
    )
    parser.add_argument(
        "--fixtures",
        nargs="*",
        default=["python", "typescript"],
        help="Fixtures to evaluate: python, typescript, or both (default: both)",
    )
    parser.add_argument(
        "--types",
        nargs="*",
        default=None,
        help="Test types to run: search, context, callers, callees, impact (default: all)",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Show detailed output per test case",
    )
    args = parser.parse_args()

    # Determine repo root (script is at __tests__/evaluation/eval_mcp.py)
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent.parent

    # Build binary if not specified
    binary = args.binary
    if binary is None:
        print("Building codegraph...")
        result = subprocess.run(
            ["cargo", "build", "-p", "codegraph-cli"],
            cwd=repo_root,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"Build failed:\n{result.stderr}")
            sys.exit(1)
        binary = str(repo_root / "target" / "debug" / "codegraph")

    # Select fixtures
    fixtures: list[Fixture] = []
    for name in args.fixtures:
        if name == "python":
            fixtures.append(PYTHON_FIXTURE)
        elif name == "typescript":
            fixtures.append(TYPESCRIPT_FIXTURE)
        else:
            print(f"Unknown fixture: {name}")
            sys.exit(1)

    # Run evaluation
    all_results: dict[str, list[TestResult]] = {}

    for fixture in fixtures:
        fixture_abs = str(repo_root / fixture.path)

        print(f"\n--- {fixture.name} ---")
        print(f"  Indexing {fixture_abs}...")
        index_fixture(binary, fixture_abs)

        print(f"  Starting MCP server...")
        client = McpClient(binary, fixture_abs)
        client.start()

        results: list[TestResult] = []
        try:
            for tc in fixture.test_cases:
                # Filter by type if specified
                if args.types and tc.type not in args.types:
                    continue

                result = run_test_case(client, tc)
                results.append(result)

                if args.verbose:
                    status = "PASS" if result.passed else "FAIL"
                    print(f"  [{status}] {tc.id}: P={result.precision:.0%} R={result.recall:.0%} F1={result.f1:.0%}")
                    if result.true_positives:
                        print(f"    TP: {result.true_positives}")
                    if result.false_negatives:
                        print(f"    FN: {result.false_negatives}")
                    if result.error:
                        print(f"    Error: {result.error}")
        finally:
            client.stop()

        all_results[fixture.name] = results
        print_results_table(results, fixture.name)

    print_summary(all_results)

    # Exit with non-zero if any tests failed
    total_failed = sum(1 for results in all_results.values() for r in results if not r.passed)
    sys.exit(1 if total_failed > 0 else 0)


if __name__ == "__main__":
    main()
