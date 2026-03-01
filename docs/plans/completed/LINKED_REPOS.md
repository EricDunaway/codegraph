# Multi-Repository Tracking Design

> **Status:** SUPERSEDED by `CROSS_LANGUAGE_LINKING.md` — monorepo approach replaces workspace-based multi-repo design.

## Overview

Enable CodeGraph to track multiple repositories and make connections between them - specifically linking backend API endpoints to frontend API calls, and tracking shared library dependencies across repos.

## Use Cases

1. **API Contract Matching**: Match backend TypeScript AppSync resolvers with frontend Dart/Flutter GraphQL calls by operation names
2. **Shared Library Dependencies**: Track Dart/Flutter package imports across repos (path dependencies for local dev)

## Architecture

### Target Stack

| Layer | Technology | Patterns |
|-------|------------|----------|
| Backend | TypeScript | `@AppSyncQuery`, `@AppSyncMutation`, `@SubscriptionModel` decorators |
| Frontend | Dart/Flutter | GraphQL query strings with Amplify |
| Shared | Dart libraries | `pubspec.yaml` path/git dependencies |

### Workspace Configuration

Workspaces stored at user level:

```
~/.codegraph-workspaces/
└── vantage/                          # Workspace name
    ├── codegraph-workspace.json      # Configuration
    └── codegraph.db                  # Unified database (all repos in one DB)
```

**`codegraph-workspace.json`:**
```json
{
  "name": "vantage",
  "repos": [
    {
      "name": "core-platform-backend",
      "path": "/Users/edunaway/Estimations/Apps/core-platform-backend",
      "role": "backend"
    },
    {
      "name": "guest-vue-front-end",
      "path": "/Users/edunaway/Estimations/Apps/guest-vue-front-end",
      "role": "frontend"
    },
    {
      "name": "shared-dart-lib",
      "path": "/Users/edunaway/Estimations/Apps/shared-dart-lib",
      "role": "library"
    }
  ],
  "connections": {
    "appsync": {
      "backend_decorators": ["AppSyncQuery", "AppSyncMutation"],
      "subscription_decorator": "SubscriptionModel",
      "method_name_field": "methodName"
    }
  }
}
```

### Database Schema Additions

```sql
-- Track repos in workspace
CREATE TABLE repos (
    id TEXT PRIMARY KEY,        -- e.g., "core-platform-backend"
    path TEXT NOT NULL,
    role TEXT,                  -- "backend", "frontend", "library"
    indexed_at INTEGER
);

-- Cross-repo edges (separate from intra-repo edges for query performance)
CREATE TABLE cross_repo_edges (
    id INTEGER PRIMARY KEY,
    source_repo TEXT NOT NULL,
    source_node TEXT NOT NULL,
    target_repo TEXT NOT NULL,
    target_node TEXT NOT NULL,
    kind TEXT NOT NULL,         -- "appsync_query", "appsync_mutation", "appsync_subscription", "package_import"
    confidence REAL,            -- 0.0-1.0 match confidence
    metadata TEXT,              -- JSON with match details
    FOREIGN KEY (source_repo) REFERENCES repos(id),
    FOREIGN KEY (target_repo) REFERENCES repos(id)
);

CREATE INDEX idx_cross_edges_source ON cross_repo_edges(source_repo, source_node);
CREATE INDEX idx_cross_edges_target ON cross_repo_edges(target_repo, target_node);
CREATE INDEX idx_cross_edges_kind ON cross_repo_edges(kind);
```

---

## Cross-Repo Edge Detection

### AppSync Connection Patterns

| Backend Decorator | Name Source | Frontend Match Pattern |
|-------------------|-------------|------------------------|
| `@AppSyncQuery({ methodName: "X" })` | Explicit `methodName` field | `X(input: ...)` in GraphQL query |
| `@AppSyncMutation({ methodName: "X" })` | Explicit `methodName` field | `X(input: ...)` in GraphQL mutation |
| `@SubscriptionModel()` on class `FooSubscriptionModel` | Derived: `subscribeToFoo` | `subscribeToFoo(...)` in GraphQL subscription |

### Additional Backend Decorators to Track

| Decorator | Purpose | Notes |
|-----------|---------|-------|
| `@SubscriptionModelField()` | Fields on subscription models | Captures field schema for type matching |
| `@SubscriptionModelAuthGuard()` | Auth configuration for subscriptions | Useful for security analysis |
| `@IAMPolicyAccessPublishSubscriptionModel()` | Lambda permission to publish to subscription | Creates mutation behind the scenes |
| `@Entity()` | DynamoDB entity definition | `streamEnabled`, `prismaEnabled` options |
| `@EntityAppSyncType()` | AppSync type exposed via GraphQL | Type definitions |
| `@GlobalSecondaryIndex()` | DynamoDB GSI definitions | Index tracking |
| `@FlexConnect()` | Flex connect integrations | Third-party integrations |
| `@FlexConnectConfiguration()` | Flex connect config class | Configuration schema |
| `@FlexConnectConfigurationField()` | Flex connect config fields | Field definitions |
| `@OpenSearchIndex()` | OpenSearch index definitions | Search indexes |
| `@KMSKey()` | KMS key definitions | Encryption keys |

### Real-World File Path Examples

These are actual file paths from the codebase demonstrating the cross-repo connection points:

| Role | File Path |
|------|-----------|
| Backend Query | `/Users/edunaway/Estimations/Apps/core-platform-backend/projects/services/lambdas/guests-search-entitlement-instances/src/index.ts` |
| Backend Subscription | `/Users/edunaway/Estimations/Apps/core-platform-backend/projects/subscriptions/src/models/guests-payment-method-added.subscription-model.ts` |
| Frontend Query | `/Users/edunaway/Estimations/Apps/guest-vue-front-end/lib/screens/entitlements/repository/entitlements_repository.dart` |
| Frontend Subscription | `/Users/edunaway/Estimations/Apps/guest-vue-front-end/lib/services/implementations/payment_method_subscription_service.dart` |

### Backend Decorator Examples

**Query** (`/Users/edunaway/Estimations/Apps/core-platform-backend/projects/services/lambdas/guests-search-entitlement-instances/src/index.ts`):
```typescript
@AppSyncQuery({
  methodName: "guests_searchEntitlementInstances",
  input: GuestsSearchEntitlementInstancesInput,
  output: GuestsSearchEntitlementInstancesResultType,
})
```

**Subscription** (`/Users/edunaway/Estimations/Apps/core-platform-backend/projects/subscriptions/src/models/guests-payment-method-added.subscription-model.ts`):
```typescript
@SubscriptionModel()
@SubscriptionModelAuthGuard(SubscriptionsGuestsAuthGuard, {
  configuration: {
    filterMode: "organizationId",
    organizationIdField: "organizationId",
  },
})
export class GuestsPaymentMethodAddedSubscriptionModel {
  @SubscriptionModelField({ isRequired: true, filterMode: SubscriptionModelFilterMode.Required })
  organizationId!: string;

  @SubscriptionModelField({ isRequired: true, filterMode: SubscriptionModelFilterMode.Required })
  guestId!: string;

  @SubscriptionModelField({ isRequired: true, filterMode: SubscriptionModelFilterMode.Optional })
  paymentMethodId!: string;

  @SubscriptionModelField({ isRequired: true, filterMode: SubscriptionModelFilterMode.Optional })
  type!: string; // "CARD", "BANK_ACCOUNT", etc.

  @SubscriptionModelField()
  lastFour?: string;

  @SubscriptionModelField()
  brand?: string; // "VISA", "MASTERCARD", "AMEX", etc.

  @SubscriptionModelField()
  expiration?: string;

  @SubscriptionModelField()
  nameOnCard?: string;

  @SubscriptionModelField({ isRequired: true })
  isDefault!: string; // "true" or "false"

  @SubscriptionModelField()
  flexConnectId?: string;
}
```

### Frontend GraphQL Examples

**Query** (`/Users/edunaway/Estimations/Apps/guest-vue-front-end/lib/screens/entitlements/repository/entitlements_repository.dart`):
```dart
query GuestsSearchEntitlementAndVoucherInstancesService {
  entitlements: guests_searchEntitlementInstances(input: { isActive: $isActive, limit: 9999 }) {
    data {
      entitlementsInstances {
        description
        expirationDate
        expirationColor
        guestId
        id
        isTransferred
        name
        organizationId
        productId
        productImageKeys
        recordType
        redeemableStatus
        category
        entitlementId
        transferredToGuestId
      }
    }
    error
    message
    status
  }
  vouchers: guests_listVoucherInstances(input: { isRedeemed: ${!isActive}, limit: 9999 }) {
    data {
      voucherInstances {
        id
        organizationId
        name
        guestId
        productId
        redeemedAt
        productImageKeys
        productCategories
        recordType
        voucherId
      }
    }
    error
    message
    status
  }
}
```

**Subscription** (`/Users/edunaway/Estimations/Apps/guest-vue-front-end/lib/services/implementations/payment_method_subscription_service.dart`):
```dart
final subscriptionQuery = '''
  subscription SubscribeToPaymentMethodAdded {
    subscribeToGuestsPaymentMethodAdded(
      organizationId: "$organizationId",
      guestId: "$guestId"
    ) {
      organizationId
      guestId
      paymentMethodId
      type
      lastFour
      brand
      expiration
      nameOnCard
      isDefault
      flexConnectId
    }
  }
''';
```

### New Edge Kinds

```rust
pub enum EdgeKind {
    // ... existing edges ...

    /// Cross-repo: Frontend calls backend AppSync query
    AppSyncQueryCall,
    /// Cross-repo: Frontend calls backend AppSync mutation
    AppSyncMutationCall,
    /// Cross-repo: Frontend subscribes to backend subscription
    AppSyncSubscription,
    /// Cross-repo: Dart package imports another repo (via pubspec.yaml path dep)
    PackageImport,
}
```

### Detection Algorithm

```
Phase 1: Index Backend Repo
├── Extract @AppSyncQuery nodes
│   └── Capture methodName from decorator args → store in node.metadata
├── Extract @AppSyncMutation nodes
│   └── Capture methodName from decorator args → store in node.metadata
└── Extract @SubscriptionModel classes
    └── Derive subscription name: "subscribeTo" + ClassName - "SubscriptionModel"
    └── e.g., GuestsPaymentMethodAddedSubscriptionModel → subscribeToGuestsPaymentMethodAdded

Phase 2: Index Frontend Repo
├── Scan for GraphQL query strings (triple-quoted strings containing query/mutation/subscription)
├── Parse GraphQL operations
│   ├── Extract operation type (query, mutation, subscription)
│   └── Extract resolver/field names being called
└── Store as nodes with kind=GraphQLOperation, metadata contains resolver names

Phase 3: Cross-Repo Resolution (after all repos indexed)
├── For each frontend GraphQL operation:
│   ├── Extract resolver names from query body
│   ├── Search backend nodes for matching methodName or derived subscription name
│   └── Create cross_repo_edge with confidence score
└── For each pubspec.yaml path dependency:
    ├── Check if target path is a tracked repo
    └── Create cross_repo_edge (kind=PackageImport)
```

### Confidence Scoring

| Match Type | Confidence | Example |
|------------|------------|---------|
| Exact methodName match | 1.0 | `guests_searchEntitlementInstances` ↔ `methodName: "guests_searchEntitlementInstances"` |
| Subscription name derivation | 0.95 | `subscribeToGuestsPaymentMethodAdded` ↔ `GuestsPaymentMethodAddedSubscriptionModel` |
| Fuzzy match (Levenshtein ≤ 3) | 0.7 | Handles typos or minor variations |
| Semantic similarity (embeddings) | 0.5-0.9 | Fallback for non-standard naming |

### Cross-Repo Connection Map (Real Examples)

These are the actual connections that would be detected between the repos:

| Frontend File | GraphQL Resolver | Backend File | Match Type |
|---------------|------------------|--------------|------------|
| `guest-vue-front-end/.../entitlements_repository.dart` | `guests_searchEntitlementInstances` | `core-platform-backend/.../guests-search-entitlement-instances/src/index.ts` | Exact methodName |
| `guest-vue-front-end/.../entitlements_repository.dart` | `guests_listVoucherInstances` | *(backend file TBD)* | Exact methodName |
| `guest-vue-front-end/.../payment_method_subscription_service.dart` | `subscribeToGuestsPaymentMethodAdded` | `core-platform-backend/.../guests-payment-method-added.subscription-model.ts` | Subscription derivation |

---

## Shared Library Detection

### pubspec.yaml Path Dependencies

```yaml
# In frontend app's pubspec.yaml
dependencies:
  shared_models:
    path: ../shared-dart-lib  # Local development
  # OR
  shared_models:
    git:
      url: https://github.com/org/shared-dart-lib
      ref: main
```

### Detection

1. Parse `pubspec.yaml` in each Dart repo
2. Extract path dependencies
3. Resolve paths relative to repo root
4. Check if resolved path matches another repo in workspace
5. Create `PackageImport` cross-repo edge

### Warning System

During indexing, warn if:
- A `pubspec.yaml` path dependency points to a directory not in the workspace
- A repo imports from another repo that hasn't been indexed yet

```
⚠ Warning: shared_models (../shared-dart-lib) is not in workspace
  Hint: Add with `codegraph workspace add ../shared-dart-lib`
```

---

## CLI Commands

```bash
# Create a new workspace
codegraph workspace init vantage

# Add repos to workspace
codegraph workspace add /path/to/backend --role backend
codegraph workspace add /path/to/frontend --role frontend
codegraph workspace add /path/to/shared-lib --role library

# Index all repos in workspace
codegraph workspace index

# Show workspace status
codegraph workspace status

# Query across repos
codegraph workspace query "guests_searchEntitlementInstances"

# Show cross-repo connections for a symbol
codegraph workspace connections guests_searchEntitlementInstances

# Show what frontend code calls a backend resolver
codegraph workspace callers guests_searchEntitlementInstances --cross-repo
```

---

## MCP Tools

New tools for multi-repo workspaces:

| Tool | Description |
|------|-------------|
| `codegraph_workspace_status` | List repos and cross-repo edge counts |
| `codegraph_cross_repo_callers` | Find frontend code that calls a backend resolver |
| `codegraph_cross_repo_callees` | Find backend resolvers called by frontend code |
| `codegraph_api_contract` | Show full API contract (backend resolver + all frontend usages) |

---

## Implementation Phases

### Phase 1: Workspace Foundation
- [ ] Add `repos` and `cross_repo_edges` tables to schema
- [ ] Implement `codegraph-workspace.json` config parsing
- [ ] CLI commands: `workspace init`, `workspace add`, `workspace status`
- [ ] Workspace-level database initialization

### Phase 2: Backend Decorator Extraction
- [ ] Enhance TypeScript extractor to capture decorator arguments
- [ ] Extract `@AppSyncQuery`, `@AppSyncMutation` with `methodName`
- [ ] Extract `@SubscriptionModel` classes and derive subscription names
- [ ] Store in node metadata

### Phase 3: Frontend GraphQL Parsing
- [ ] Detect GraphQL query strings in Dart files (triple-quoted, contains query/mutation/subscription)
- [ ] Parse GraphQL operations to extract resolver names
- [ ] Create nodes for GraphQL operations with resolver references in metadata

### Phase 4: Cross-Repo Resolution
- [ ] Implement cross-repo edge creation after indexing
- [ ] Match frontend GraphQL calls to backend resolvers
- [ ] Implement confidence scoring
- [ ] Add `--cross-repo` flag to existing query commands

### Phase 5: Dart Package Dependencies
- [ ] Parse `pubspec.yaml` for path/git dependencies
- [ ] Resolve paths and match to workspace repos
- [ ] Create `PackageImport` edges
- [ ] Implement missing dependency warnings

### Phase 6: MCP Tools
- [ ] Add workspace-aware MCP tools
- [ ] Cross-repo traversal in existing tools

---

## Example Queries

### "What frontend code calls the guests search resolver?"

```sql
SELECT
    cre.source_repo,
    n.file_path,
    n.name,
    cre.confidence
FROM cross_repo_edges cre
JOIN nodes n ON n.id = cre.source_node
WHERE cre.target_node IN (
    SELECT id FROM nodes
    WHERE metadata LIKE '%"methodName":"guests_searchEntitlementInstances"%'
)
AND cre.kind = 'appsync_query';
```

### "What backend resolvers does this Dart file use?"

```sql
-- Find all backend resolvers called by entitlements_repository.dart
SELECT
    cre.target_repo,
    tn.name as resolver_name,
    tn.file_path as backend_file,
    cre.kind,
    cre.confidence
FROM cross_repo_edges cre
JOIN nodes sn ON sn.id = cre.source_node
JOIN nodes tn ON tn.id = cre.target_node
WHERE sn.file_path = 'lib/screens/entitlements/repository/entitlements_repository.dart'
AND cre.source_repo = 'guest-vue-front-end';

-- Expected results:
-- | target_repo            | resolver_name                      | backend_file                                                                  | kind          | confidence |
-- |------------------------|------------------------------------|-------------------------------------------------------------------------------|---------------|------------|
-- | core-platform-backend  | guests_searchEntitlementInstances  | projects/services/lambdas/guests-search-entitlement-instances/src/index.ts    | appsync_query | 1.0        |
-- | core-platform-backend  | guests_listVoucherInstances        | projects/services/lambdas/guests-list-voucher-instances/src/index.ts          | appsync_query | 1.0        |
```

### "What subscriptions does a frontend service use?"

```sql
-- Find subscriptions used by payment_method_subscription_service.dart
SELECT
    cre.target_repo,
    tn.name as subscription_class,
    tn.file_path as backend_file,
    json_extract(cre.metadata, '$.derived_name') as subscription_name,
    cre.confidence
FROM cross_repo_edges cre
JOIN nodes sn ON sn.id = cre.source_node
JOIN nodes tn ON tn.id = cre.target_node
WHERE sn.file_path = 'lib/services/implementations/payment_method_subscription_service.dart'
AND cre.source_repo = 'guest-vue-front-end'
AND cre.kind = 'appsync_subscription';

-- Expected results:
-- | target_repo           | subscription_class                      | backend_file                                                                          | subscription_name                  | confidence |
-- |-----------------------|-----------------------------------------|---------------------------------------------------------------------------------------|------------------------------------|------------|
-- | core-platform-backend | GuestsPaymentMethodAddedSubscriptionModel | projects/subscriptions/src/models/guests-payment-method-added.subscription-model.ts | subscribeToGuestsPaymentMethodAdded | 0.95       |
```

---

## Future Enhancements (TODO)

- [ ] Schema diffing: Detect when backend changes would break frontend
- [ ] Auto-discovery: Scan pubspec.yaml/package.json for dependencies and suggest adding repos
- [ ] GraphQL schema extraction: Parse `.graphql` schema files for type definitions
- [ ] Type matching: Match request/response types across TypeScript ↔ Dart
- [ ] Git integration: Track which commits changed cross-repo contracts
