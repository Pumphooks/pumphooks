# @pump-hooks/execution-lab

Runs Solana transactions against real program binaries in [LiteSVM](https://github.com/LiteSVM/litesvm)
(Apache-2.0) and returns an execution trace.

| Field | Provenance | Source |
| --- | --- | --- |
| `status`, `error` | observed | transaction result |
| `instructions` | observed | compiled message (accounts with signer / writable flags) |
| `cpiTree` | observed | inner instructions with stack heights |
| `invocations` | observed | runtime logs: invoke depth, CU consumed, success / failure |
| `computeUnits` | observed | total, and per top-level instruction from logs |
| `accountDiffs` | observed | every message account read before and after: lamports, owner, changed byte ranges |
| `tokenBalances` | observed | SPL Token / Token-2022 amounts before and after |
| `logs`, `returnData` | observed | runtime |
| `hookSteps` | **reconstructed** | the installed hook stack read from the policy account before execution, the kernel's in-order evaluation, and its rejection log line. Builtins do not log individually. |

`trace.provenance` carries this table, so UIs can label each section.

```ts
import { ExecutionLab, exportRepro, replay, renderTrace } from "@pump-hooks/execution-lab";

const lab = new ExecutionLab();
lab.addProgram({ id: "FZBB…", path: "target/deploy/token_hooks.so", name: "token-hooks" });
lab.loadAccountDir("tests/fixtures/venues/mainnet/dlmm"); // `solana account --output json` files
const trace = await lab.execute({ payer, signers, instructions, label: "transfer" });
console.log(renderTrace(trace));

const bundle = exportRepro(lab, trace, { pathRoot: "." }); // no secret keys; program sha256 pinned
const { identical } = await replay(bundle, { pathRoot: "." }); // status, error, CU and log digest
```

Hook-order resolvers exist for `token-hooks` (`Execute`) and `hook-router` (`Route`). Pass
`resolvers` to add others.

Tests: `pnpm test`. The token-hooks tests need `scripts/build_programs.sh` first; they are skipped
when `target/deploy/token_hooks.so` is missing, and CI runs them after the SBF build
(`PUMPHOOKS_SBF=1`).
