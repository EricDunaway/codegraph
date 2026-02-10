#!/usr/bin/env python3
"""Unit test for extract_symbols_from_context against known output format."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from eval_mcp import extract_symbols_from_context, normalize

# Simulated context output based on the formatter.rs format_markdown method
# for "debug why users cannot log in" against the Python fixture
PYTHON_LOGIN_CONTEXT = """# Context: debug why users cannot log in

**8 symbols** across **3 files**

## Files

- `auth.py` (5 symbols)
- `database.py` (1 symbols)
- `models.py` (1 symbols)
- `validation.py` (1 symbols)

## Symbols

### method `auth.py::AuthService.login`

- **File:** `auth.py:57:68`
- **Signature:** `def login(self, email: str, password: str) -> Optional[str]`

> Authenticate user and return token.

```python
    def login(self, email: str, password: str) -> Optional[str]:
        \"\"\"Authenticate user and return token.\"\"\"
        user = db.get_user_by_email(email)
        if not user:
            return None

        if not verify_password(password, user.password_hash):
            return None

        token = generate_token()
        self.tokens[token] = user.id
        return token
```

### function `auth.py::verify_password`

- **File:** `auth.py:20:24`
- **Signature:** `def verify_password(password: str, password_hash: str) -> bool`

> Verify a password against its hash.

```python
def verify_password(password: str, password_hash: str) -> bool:
    \"\"\"Verify a password against its hash.\"\"\"
    salt, stored_hash = password_hash.split(":")
    hash_obj = hashlib.sha256((password + salt).encode())
    return hash_obj.hexdigest() == stored_hash
```

### function `auth.py::hash_password`

- **File:** `auth.py:13:17`
- **Signature:** `def hash_password(password: str) -> str`

> Hash a password for storage.

```python
def hash_password(password: str) -> str:
    \"\"\"Hash a password for storage.\"\"\"
    salt = secrets.token_hex(16)
    hash_obj = hashlib.sha256((password + salt).encode())
    return f"{salt}:{hash_obj.hexdigest()}"
```

### function `auth.py::generate_token`

- **File:** `auth.py:27:29`
- **Signature:** `def generate_token() -> str`

> Generate a secure random token.

```python
def generate_token() -> str:
    \"\"\"Generate a secure random token.\"\"\"
    return secrets.token_urlsafe(32)
```

### class `auth.py::AuthService`

- **File:** `auth.py:32:77`

```python
class AuthService:
    def __init__(self):
        self.tokens: dict = {}
```

### class `models.py::User`

- **File:** `models.py:5:12`

```python
class User:
    id: str
    email: str
    name: str
    password_hash: str
    created_at: datetime
```

### method `database.py::Database.get_user_by_email`

- **File:** `database.py:15:20`
- **Signature:** `def get_user_by_email(self, email: str) -> Optional[User]`

```python
    def get_user_by_email(self, email: str) -> Optional[User]:
        for user in self.users.values():
            if user.email == email:
                return user
        return None
```

### function `validation.py::validate_email`

- **File:** `validation.py:5:8`
- **Signature:** `def validate_email(email: str) -> bool`

```python
def validate_email(email: str) -> bool:
    \"\"\"Validate email format.\"\"\"
    return "@" in email and "." in email
```

## Relationships

"""

# Test: Python login bug context
print("=" * 60)
print("TEST: Python login context")
print("=" * 60)
symbols = extract_symbols_from_context(PYTHON_LOGIN_CONTEXT)
norm_symbols = {normalize(s) for s in symbols}

expected = ["login", "verify_password", "get_user_by_email", "User", "hash_password"]
found = []
missed = []
for e in expected:
    if normalize(e) in norm_symbols:
        found.append(e)
    else:
        missed.append(e)

print(f"Extracted symbols ({len(symbols)}): {sorted(symbols)}")
print(f"Expected: {expected}")
print(f"Found: {found}")
print(f"Missed: {missed}")
print(f"Recall: {len(found)}/{len(expected)} = {len(found)/len(expected):.0%}")
print()

# Test TypeScript order creation context
TS_ORDER_CONTEXT = """# Context: understand how orders are created and validated

**6 symbols** across **3 files**

## Files

- `src/order.ts` (3 symbols)
- `src/utils/validation.ts` (1 symbols)
- `src/types.ts` (2 symbols)

## Symbols

### method `src/order.ts::OrderService.createOrder`

- **File:** `src/order.ts:13:56`
- **Signature:** `async createOrder(token: string, items: OrderItem[]): Promise<Order>`

> Create a new order

```typescript
  async createOrder(token: string, items: OrderItem[]): Promise<Order> {
    const userId = await authService.validateToken(token);
    if (!userId) {
      throw new Error('Invalid or expired token');
    }

    // Validate items
    for (const item of items) {
      if (!validateQuantity(item.quantity)) {
        throw new Error(`Invalid quantity for product ${item.productId}`);
      }

      const product = await db.findProductById(item.productId);
      if (!product) {
        throw new Error(`Product not found: ${item.productId}`);
      }
    }

    // Calculate total
    const total = paymentService.calculateTotal(items);

    // Create order
    const order: Order = {
      id: generateOrderId(),
      userId,
      items,
      total,
      status: 'pending',
      createdAt: new Date(),
    };

    await db.createOrder(order);

    // Update stock
    for (const item of items) {
      await db.updateProductStock(item.productId, item.quantity);
    }

    return order;
  }
```

### class `src/order.ts::OrderService`

- **File:** `src/order.ts:12:113`

```typescript
export class OrderService {
```

### function `src/utils/validation.ts::validateQuantity`

- **File:** `src/utils/validation.ts:10:15`
- **Signature:** `export function validateQuantity(quantity: number): boolean`

```typescript
export function validateQuantity(quantity: number): boolean {
  return Number.isInteger(quantity) && quantity > 0;
}
```

### interface `src/types.ts::Order`

- **File:** `src/types.ts:20:30`

```typescript
export interface Order {
  id: string;
  userId: string;
  items: OrderItem[];
  total: number;
  status: 'pending' | 'paid' | 'shipped' | 'delivered' | 'cancelled';
  createdAt: Date;
}
```

### interface `src/types.ts::OrderItem`

- **File:** `src/types.ts:32:36`

```typescript
export interface OrderItem {
  productId: string;
  quantity: number;
  price: number;
}
```

### function `src/utils/crypto.ts::generateOrderId`

- **File:** `src/utils/crypto.ts:15:20`
- **Signature:** `export function generateOrderId(): string`

```typescript
export function generateOrderId(): string {
  return crypto.randomUUID();
}
```

## Relationships

"""

print("=" * 60)
print("TEST: TypeScript order creation context")
print("=" * 60)
symbols = extract_symbols_from_context(TS_ORDER_CONTEXT)
norm_symbols = {normalize(s) for s in symbols}

expected = ["createOrder", "validateQuantity", "findProductById", "Order", "OrderItem"]
found = []
missed = []
for e in expected:
    if normalize(e) in norm_symbols:
        found.append(e)
    else:
        missed.append(e)

print(f"Extracted symbols ({len(symbols)}): {sorted(symbols)}")
print(f"Expected: {expected}")
print(f"Found: {found}")
print(f"Missed: {missed}")
print(f"Recall: {len(found)}/{len(expected)} = {len(found)/len(expected):.0%}")

# Summary
print("\n" + "=" * 60)
print("All tests complete")
