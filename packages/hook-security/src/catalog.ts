// Attack categories and the executed tests that exercise each one. Evidence names are checked to
// exist by test/catalog.test.ts. Covered means "a test attempts this and the attempt fails", not
// "this class of bug cannot exist". This is not an audit.

import type { AttackId } from "./targets/wrapper.js";

export interface Evidence {
  /** Rust `#[test] fn` name under tests/ or bench/, or a wrapper-target attack / invariant. */
  test: string;
  kind: "rust" | "fuzz-attack" | "fuzz-invariant";
  level: "unit" | "localnet" | "mainnet-binary";
}

export interface Category {
  id: string;
  title: string;
  threat: string;
  evidence: Evidence[];
  /** Known gaps, stated plainly. */
  gaps?: string;
}

const rust = (test: string, level: Evidence["level"] = "localnet"): Evidence => ({ test, kind: "rust", level });
const attack = (test: AttackId): Evidence => ({ test, kind: "fuzz-attack", level: "localnet" });
const invariant = (test: string): Evidence => ({ test, kind: "fuzz-invariant", level: "localnet" });

export const CATEGORIES: Category[] = [
  {
    id: "missing-signer",
    title: "Missing or forged signer",
    threat: "An instruction acts for an account whose owner did not sign.",
    evidence: [rust("initialize_requires_the_upgrade_authority"), rust("credit_operations_require_hook_authority"), attack("wrap-from-victim"), attack("unwrap-victim")],
  },
  {
    id: "authority-confusion",
    title: "Wrong authority",
    threat: "A signer that is not the configured authority changes state it does not control.",
    evidence: [rust("status_only_moves_forward_and_only_by_authority"), rust("builtins_only_from_admin_with_exact_claims"), rust("private_policies_only_apply_to_their_authority"), attack("direct-mint")],
  },
  {
    id: "account-substitution",
    title: "Account substitution",
    threat: "A caller passes a look-alike account (vault, pool, policy, token account) in place of the bound one.",
    evidence: [
      rust("execute_rejects_protected_accounts_self_cpi_and_substitution"),
      rust("adapter_rejects_disabled_venues_unknown_instructions_and_substituted_accounts"),
      rust("wrong_hook_accounts_are_rejected"),
      attack("fake-vault"),
      attack("foreign-policy"),
    ],
  },
  {
    id: "forged-callback",
    title: "Forged hook invocation",
    threat: "A hook is called outside the operation it guards, to consume limits or fake approvals.",
    evidence: [rust("forged_callbacks_are_refused_by_the_hook"), attack("direct-execute")],
  },
  {
    id: "reentrancy",
    title: "Reentrancy and self-CPI",
    threat: "A hook or executed instruction calls back into the engine mid-operation.",
    evidence: [rust("hook_cannot_reenter_core"), rust("execute_rejects_protected_accounts_self_cpi_and_substitution")],
  },
  {
    id: "asset-access",
    title: "Hook access to user assets",
    threat: "A hook moves tokens or lamports that belong to the user.",
    evidence: [rust("hook_cannot_spend_trader_tokens"), rust("sol_taken_from_the_wallet_is_measured_and_capped"), rust("execute_blocks_token_side_channels_and_limits_tokens")],
  },
  {
    id: "unbacked-mint",
    title: "Unbacked minting",
    threat: "Wrapped or receipt tokens are created without the matching deposit.",
    evidence: [rust("wrapper_backs_every_wrapped_token_and_runs_the_hooks"), attack("direct-mint"), invariant("reserves-cover-supply"), invariant("fee-free-underlying-is-exactly-backed")],
  },
  {
    id: "custody-withdrawal",
    title: "Unauthorized withdrawal / trapped funds",
    threat: "Custodied funds leave without the owner, or policies trap them.",
    evidence: [rust("pool_policies_cannot_gate_withdrawals"), rust("pause_never_blocks_withdrawals"), rust("timelocked_withdrawals_need_aged_proposals_and_cannot_be_replayed"), invariant("underlying-conserved")],
  },
  {
    id: "policy-bypass",
    title: "Mandatory policy removal or reset",
    threat: "An authority removes or weakens a mandatory entry, or resets its state, without the delay.",
    evidence: [
      rust("mandatory_policy_cannot_be_removed_without_an_aged_proposal"),
      rust("mandatory_policy_entries_change_only_after_the_delay"),
      rust("resubmitting_the_stack_does_not_reset_spending"),
    ],
  },
  {
    id: "code-substitution",
    title: "Hook binary substitution",
    threat: "A hook program is upgraded after install and keeps running with new code.",
    evidence: [rust("pinned_hook_stops_on_redeploy"), rust("pinning_requires_the_binary_the_version_describes"), rust("program_versions_record_loader_facts")],
    gaps: "Upgradeable hooks that are not pinned run whatever is deployed; the registry records the upgrade authority so clients can show it.",
  },
  {
    id: "replay",
    title: "Replay and stale state",
    threat: "A signed intent, proposal or session is reused after it should expire.",
    evidence: [rust("deadline_and_receipt_nonce_replay"), rust("proposals_go_stale_on_governance_changes"), rust("sessions_are_scoped_expire_and_die_with_the_epoch")],
  },
  {
    id: "price-manipulation",
    title: "Slippage, sandwich and price manipulation",
    threat: "A trade settles at a manipulated price or worse than the user's bound.",
    evidence: [
      rust("measured_min_out_and_max_in_are_enforced_by_the_router"),
      rust("slot_limit_refuses_same_slot_round_trips"),
      rust("price_impact_policy_reads_live_reserves"),
      rust("price_range_policy_reverts_swaps_that_leave_the_range"),
    ],
    gaps: "Router hooks are bypassable: the same pool remains callable directly (see max_size_caps_buys_and_venue_stays_directly_callable).",
  },
  {
    id: "fee-accounting",
    title: "Fee and transfer-fee accounting",
    threat: "Fees are charged above bounds, or nominal amounts are trusted instead of measured deltas.",
    evidence: [rust("wrapper_refuses_unsafe_underlyings_and_measures_transfer_fees"), rust("after_swap_fee_is_bounded_by_trader_limit"), rust("protocol_fee_share_is_capped_and_paid_out")],
  },
  {
    id: "registry-poisoning",
    title: "Initialization front-running and registry poisoning",
    threat: "Someone initializes a program first, squats a slug, or installs a version that does not match its claims.",
    evidence: [rust("initialization_cannot_be_front_run"), rust("slugs_are_validated_and_unique"), rust("installs_must_match_the_version"), rust("audits_require_named_reviewers")],
  },
  {
    id: "fail-open",
    title: "Fail-open, partial execution and compute exhaustion",
    threat: "An error path allows the operation, a failed transaction leaves partial state, or a stack exceeds compute.",
    evidence: [
      rust("silent_hook_is_an_error_not_an_allow"),
      rust("compact_policy_fits_in_a_transaction"),
      rust("each_measured_builtin_actually_evaluates"),
      invariant("failed-operations-change-nothing"),
      invariant("honest-operations-behave-as-specified"),
      attack("transfer-no-hook"),
    ],
  },
];
