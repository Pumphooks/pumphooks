# Security

Pump Hooks is **unaudited** and is **not deployed on mainnet**. Do not use it with funds you
cannot afford to lose. The full threat model (T1–T29) is in
[docs/threat-model.md](docs/threat-model.md).

## Reporting

Please report vulnerabilities privately to the maintainers rather than opening a public issue.
Include:

- the affected component (program, runtime, SDK, web);
- a reproduction (a LiteSVM test is ideal);
- the impact.

## Guarantees the design aims for

- **No custody, no delegates.**
  - Users sign each routed trade with their own wallet.
  - The router never holds user tokens between transactions.
  - There is no token delegate or standing approval.
- **Hooks cannot move user assets.**
  - First-party modules can only allow or deny a trade, update their own 48-byte slot state,
    update the trader's per-market ledger (XP, points), and emit events.
  - Capabilities are granted at install and enforced by the executor.
- **Fail-closed.**
  - Any of these reverts the whole transaction: a hook error, a bad config, an unexpected venue
    layout, a substituted account, a paused market or a disabled module.
- **Executed amounts are measured.** After-hooks see balance deltas observed around the Pump CPI,
  not the user's inputs.
- **Bounded policies.**
  - The Policy VM has a fixed instruction budget (8 conditions, 4 actions, 4 registers).
  - It has no loops and makes no external calls.
  - Policies are validated when written; evaluation is bounded by construction.
- **SENTIENT is restrict-only.**
  - Signed regime signals need the market's configured Ed25519 signer, a fresh nonce and a
    bounded TTL.
  - They can only restrict routed trading, and every regime expires.
  - The off-chain engine never signs for users.
- **No redirected fees.**
  - Pump creator fees and protocol fees are never touched.
  - Treasury Router only describes how explicitly deposited funds would be split.
- **Kill switch.** The global authority can pause routing or disable individual modules. It
  cannot move funds.

## Known limitations

- **Bypass.** Hooks bind only trades routed through Pump Hooks. Direct trades on Pump or PumpSwap
  are unaffected; a test pins this.
- **Pump upgrades.**
  - Pump and PumpSwap can change account layouts or instruction sets.
  - The adapter is byte-checked against the official client and tested against the deployed
    devnet binaries.
  - Unexpected layouts fail closed, which may halt routing until the adapter is updated.
- **Pre-trade sizes are estimates.**
  - Before-hooks see constant-product estimates.
  - Volume Guard re-checks observed amounts after the trade.
- **Holder XP is anti-wash, not sybil-proof.** Many wallets can each earn capped XP.
- **External CPI hooks are not enabled in V1.** Only first-party modules execute.
- **The strategy vault is not deployed.** TWAP execution and treasury intents are recorded but
  not acted on.

## Operational rules for this repository

- Never commit private keys, seed phrases, RPC URLs with API keys, or other secrets. Devnet test
  keys live in the git-ignored `.keys/`.
- Tooling never reads `~/.config/solana`; RPC and keypair paths are always passed explicitly.
- No mainnet deployment without explicit approval from the project owner.
