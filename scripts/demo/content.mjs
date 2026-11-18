// Mock documents, diff and terminal text for the demo server (scripts/demo/server.mjs).
export const spec = `---
stage: spec
---
# Invoice PDFs

## Problem
Customers ask for a PDF copy of every invoice. Today they can only read it on screen, so finance teams screenshot it.

## Requirements
1. **R1.** Every invoice page has a *Download PDF* button.
2. **R2.** The PDF shows the invoice number, the issue date, the line items, the tax and the total, in the customer's currency.
3. **R3.** A PDF is generated on first request and cached until the invoice changes.
4. **R4.** A customer can only download their own invoices.

## Acceptance
| Requirement | How it is checked |
|---|---|
| R1 | UI test: the button is on the page and downloads a file |
| R2 | Unit test: the template renders every field for a multi-currency invoice |
| R3 | Integration test: two requests, one render; an edit clears the cache |
| R4 | Integration test: another customer's invoice returns 404 |

## Out of scope
- Email delivery of the PDF.
- Custom templates per customer.

## Open questions
- Should the PDF carry the company logo, or only text?
`;

export const design = `---
stage: design
---
# Coupon codes

## Approach
Codes live in a \`coupons\` table (code, percent, expires_at, max_uses). The cart applies at most one coupon and the discount is computed on the server, never in the browser.

## Flow
\`\`\`mermaid
flowchart TD
  A[Customer enters a code] --> B{Code exists?}
  B -- no --> X[Show: unknown code]
  B -- yes --> C{Expired or used up?}
  C -- yes --> Y[Show: code no longer valid]
  C -- no --> D[Apply percent to the cart subtotal]
  D --> E[Store coupon id on the cart]
  E --> F[Recompute the total on every change]
\`\`\`

## Data
| Column | Type | Notes |
|---|---|---|
| code | text | unique, upper-cased |
| percent | int | 1 to 100 |
| expires_at | timestamp | null = never |
| max_uses | int | null = unlimited |
| used | int | incremented when the order is paid |

## Interfaces
- \`POST /cart/coupon\` with \`{code}\`: applies it, returns the cart.
- \`DELETE /cart/coupon\`: removes it.

## Open questions
- Should a coupon stack with the free-shipping threshold?
`;

export const memory = `# Memory

## State
Design written; waiting for approval.

## Decisions
- Discounts are computed on the server.

## Code map
- \`src/cart/\` holds the cart logic.

## Watch out
- The cart total is also used by the tax service.
`;

export const diff = `diff --git a/src/export/csv.ts b/src/export/csv.ts
new file mode 100644
index 0000000..3f2a9c1
--- /dev/null
+++ b/src/export/csv.ts
@@ -0,0 +1,24 @@
+import type { Order } from '../orders';
+
+const COLUMNS = ['id', 'placed_at', 'customer', 'total', 'currency'] as const;
+
+/** One CSV line per order; quotes and commas in values are escaped. */
+export function toCsv(orders: Order[]): string {
+  const lines = [COLUMNS.join(',')];
+  for (const o of orders) {
+    lines.push(COLUMNS.map((c) => escape(String(o[c] ?? ''))).join(','));
+  }
+  return lines.join('\\n') + '\\n';
+}
+
+function escape(v: string): string {
+  return /[",\\n]/.test(v) ? '"' + v.replace(/"/g, '""') + '"' : v;
+}
diff --git a/src/routes/orders.ts b/src/routes/orders.ts
index 91ab2e4..c40d7f8 100644
--- a/src/routes/orders.ts
+++ b/src/routes/orders.ts
@@ -1,9 +1,10 @@
 import { Router } from 'express';
 import { listOrders } from '../orders';
+import { toCsv } from '../export/csv';
 
 export const orders = Router();
 
 orders.get('/orders', async (req, res) => {
   res.json(await listOrders(req.user.id));
 });
+
+orders.get('/orders.csv', async (req, res) => {
+  res.type('text/csv').send(toCsv(await listOrders(req.user.id)));
+});
`;

export const terminal = [
  '\x1b[2m$ xherd task start T03\x1b[0m',
  '',
  '\x1b[1;36m●\x1b[0m Working on \x1b[1mT03: Filter chips on the search page\x1b[0m',
  '',
  '  \x1b[32m✓\x1b[0m read  src/search/filters.ts',
  '  \x1b[32m✓\x1b[0m read  src/search/SearchPage.tsx',
  '  \x1b[33m✎\x1b[0m edit  src/search/FilterChips.tsx \x1b[2m(new, 48 lines)\x1b[0m',
  '  \x1b[33m✎\x1b[0m edit  src/search/SearchPage.tsx \x1b[2m(+12 −3)\x1b[0m',
  '',
  '\x1b[1;36m●\x1b[0m Running the tests for this task',
  '  \x1b[2m$ npm test -- search\x1b[0m',
  '',
  '   \x1b[32mPASS\x1b[0m  src/search/filters.test.ts',
  '   \x1b[32mPASS\x1b[0m  src/search/FilterChips.test.tsx',
  '',
  '   Tests:  \x1b[32m14 passed\x1b[0m, 14 total',
  '',
  '\x1b[1;36m●\x1b[0m All 14 tests pass. Reporting to xherd.',
  '  \x1b[2m$ xherd task done T03 --evidence "npm test -- search: 14 passed"\x1b[0m',
  '  \x1b[32m✓\x1b[0m T03 done. Next: T04 (clear all button)',
  '',
  '\x1b[2m> \x1b[0m',
].join('\r\n');

// Output of the demo build action.
export const actionLog = [
  '> web-store@1.4.0 build',
  '> vite build',
  '',
  'vite v5.4.0 building for production...',
  '✓ 128 modules transformed.',
  'dist/index.html           0.46 kB',
  'dist/assets/index.js    212.31 kB',
  '✓ built in 2.31s',
].join('\n');
