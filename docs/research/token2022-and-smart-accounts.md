# Research: Token-2022 transfer hooks and Solana smart accounts

Inputs for two Pump Hooks programs:

1. **Token wrapper.** It holds SPL tokens in a vault and mints a 1:1 Token-2022 wrapped asset whose
   `TransferHook` extension points to our hook programs.
2. **Smart account.** It runs hook stacks: spending limit, timelock, multisig approval, conditional
   execution, session permissions and recovery.

Every claim cites source code. Paths are relative to `.ref/research/`. The repos were downloaded as
HEAD tarballs on 2026-10-20, and the commit column is the HEAD sha the GitHub API reported at that
time. Statements marked **(inferred)** are conclusions drawn from the cited code, not documented
behavior.

| Directory | Upstream | HEAD at fetch |
| --- | --- | --- |
| `solana-program_transfer-hook` | solana-program/transfer-hook | `3ca0dbcbb` (see `docs/research.md`) |
| `solana-program_token-2022` | solana-program/token-2022 | `1f7b9c2a0` (2026-10-20) |
| `crates_pinocchio-transfer-hook-interface-0.1.0` | crates.io, the version pinned in token-2022 `Cargo.lock:4760` | 0.1.0 |
| `crates_spl-tlv-account-resolution-0.11.4` | crates.io, pinned in `Cargo.lock:10578` | 0.11.4 |
| `crates_spl-type-length-value-0.9.1`, `crates_spl-list-view-0.1.1` | crates.io, pinned in `Cargo.lock:10977,10486` | — |
| `solana-developers_program-examples` | solana-developers/program-examples | HEAD |
| `raydium-io_raydium-cp-swap` | raydium-io/raydium-cp-swap | `f4b9b2634` |
| `MeteoraAg_damm-v2` | MeteoraAg/damm-v2 | `5398b5c7c` |
| `orca-so_whirlpools` | orca-so/whirlpools | `cc683ec92` |
| `agave` | anza-xyz/agave | `1c12b3ab8` |
| `Squads-Protocol_v4` | Squads-Protocol/v4 | `af94153ff` |
| `Squads-Protocol_smart-account-program` | Squads-Protocol/smart-account-program | `0e510821e` |
| `magicblock-labs_session-keys` | magicblock-labs/session-keys (the `gpl_session` program) | `91c631eef` |
| `anagrambuild_swig-wallet` | anagrambuild/swig-wallet. `anagramxyz/swig-wallet` does not exist (HTTP 404). | `dd4309e15` |

---

## PART A — Token-2022 transfer hooks

### A.0 Version note: Token-2022 now uses the pinocchio interface

The live Token-2022 processor no longer calls `spl_transfer_hook_interface::onchain::invoke_execute`.
It builds the CPI with `pinocchio_transfer_hook_interface::instructions::Execute`
(`token-2022/program/src/processor.rs:26`, `:620-629`). The privilege rules on chain are therefore
the ones in `crates_pinocchio-transfer-hook-interface-0.1.0/src/instructions/execute.rs`. The older
`spl-transfer-hook-interface` helpers (`transfer-hook/interface/src/onchain.rs`) behave the same
way and are still what third-party programs use, Orca for example.

### A.1 The `Execute` instruction

| Item | Value | Source |
| --- | --- | --- |
| Discriminator | First 8 bytes of `sha256("spl-transfer-hook-interface:execute")` = `69 25 65 c5 4b fb 66 1a` (`[105,37,101,197,75,251,102,26]`). Verified locally with `shasum`. | `transfer-hook/interface/src/instruction.rs:70-72`; `lib.rs:24`; `token-2022/clients/js/src/transferHookExtraAccountMetas.ts:629` |
| Data layout | `[0..8]` discriminator, `[8..16]` `amount: u64` LE. 16 bytes in total. | `pinocchio…/execute.rs:120-137`; `instruction.rs:126-129` |
| `amount` sentinel | Confidential transfers pass `u64::MAX`, because the amount is unknown. | `instruction.rs:31-37`; `token-2022/program/src/extension/confidential_transfer/processor.rs:824-834` |
| Account 0 | Source token account, `[]` | `execute.rs:30-37`, `:149` |
| Account 1 | Mint, `[]` | `execute.rs:150` |
| Account 2 | Destination token account, `[]` | `execute.rs:151` |
| Account 3 | Source owner or delegate (the transfer authority), `[]` | `execute.rs:152` |
| Account 4 | Validation PDA (`ExtraAccountMetaList`), `[]`. **Optional.** | `execute.rs:154-168` |
| Account 5.. | The extra accounts resolved from the validation data, in exactly the declared order | `execute.rs:224-246` |
| Other ids | `initialize-extra-account-metas` = `2b220d31a758ebeb`; `update-extra-account-metas` = `9d692a926655f1ae` (computed the same way) | `instruction.rs:76-84` |

What these mean for a hook:

- **The validation account is optional on the Token-2022 side.**
  - Token-2022 looks for the validation PDA among the remaining accounts.
  - If the PDA is absent, it **still invokes the hook with only the 4 base accounts**
    (`execute.rs:154-168`, `if let Some(validation)`).
  - Our hooks must reject a call without the PDA. The reference hook does this by indexing
    account 4 unconditionally (`transfer-hook/program/src/processor.rs:48`).
- **Outer order is free; inner order is fixed.**
  - Token-2022 finds each resolved extra account **by pubkey** in the remaining accounts
    (`execute.rs:232-236`). The order in the *outer* `transfer_checked` does not matter.
  - The hook receives the accounts in declared order, and `check_account_infos` checks exact
    positions (`spl-tlv-account-resolution/src/state.rs:242-256`).
- **The hook program must be in the outer instruction.** The runtime fails a CPI whose program
  account is not among the caller's instruction accounts, with `MissingAccount`
  (`agave/program-runtime/src/invoke_context.rs:379-392`). The off-chain helpers therefore append
  the hook program id and the validation PDA (`transfer-hook/interface/src/offchain.rs:104-110`;
  `token-2022/clients/js-legacy/src/extensions/transferHook/instructions.ts:243-245`).
- **Multisig signers leak into the hook's view.** For a multisig authority, the signer accounts
  that follow `authority` are in `account_info_iter.as_slice()`, which becomes
  `additional_accounts` (`processor.rs:493`, `:626`). Only accounts that match a resolved meta are
  forwarded to the hook.

### A.2 `ExtraAccountMetaList`

**PDA.** Seeds are `["extra-account-metas", mint]` under **the hook program id**
(`transfer-hook/interface/src/lib.rs:26-47`). The validation PDA is therefore per (hook program,
mint).

**Account layout.** The account is a TLV, and the entries are 35-byte `ExtraAccountMeta`s:

| Offset | Size | Field | Source |
| --- | --- | --- | --- |
| 0 | 8 | TLV type = `ExecuteInstruction` discriminator | `spl-type-length-value-0.9.1/src/state.rs:160-164` |
| 8 | 4 | TLV length, `u32` LE | same |
| 12 | 4 | `ListView` count, `u32` LE. There is no padding, because `ExtraAccountMeta` is all `u8` / `Bool` and so has alignment 1. | `spl-list-view-0.1.1/src/list_view.rs:50-56`; `state.rs:190-194` |
| 16 + 35·i | 35 | `ExtraAccountMeta { discriminator: u8, address_config: [u8;32], is_signer: Bool, is_writable: Bool }` | `spl-tlv-account-resolution-0.11.4/src/account.rs:121-135` |

`ExtraAccountMetaList::size_of(n)` is the TLV base length plus `ListView::size_of(n)`
(`state.rs:198-201`). That equals 16 + 35·n bytes. **(inferred)** from the two sizes above.

**`ExtraAccountMeta.discriminator`** (`account.rs:207-249`):

| Value | Meaning | `address_config` |
| --- | --- | --- |
| `0` | Literal pubkey | The 32-byte key |
| `1` | PDA of **the hook program** | Packed `Seed`s, 32 bytes at most |
| `2` | Pubkey read from data (`PubkeyData`) | `InstructionData{index}` or `AccountData{account_index, data_index}`. Always 32 bytes. (`pubkey_data.rs:15-43`) |
| `128 + i` | PDA of **an external program**, whose id is the account at index `i` | Packed `Seed`s (`account.rs:190-204`, `:219-226`) |

**Seed types** (`seeds.rs:9-28`, `:37-98`, packing `:117-153`). All seeds of one meta must fit in
32 bytes.

| Seed | Encoding | Resolves to |
| --- | --- | --- |
| `Literal { bytes }` | `1, len, bytes…` | Constant bytes |
| `InstructionData { index, length }` | `2, index, length` | `ix_data[index..index+length]`. The `Execute` data is `disc(8) ‖ amount(8)`, so the amount is `{index: 8, length: 8}`. (`account.rs:33-40`; `offchain.rs:143-147`) |
| `AccountKey { index }` | `3, index` | The pubkey of account `index` in the **full** list: 0 source, 1 mint, 2 destination, 3 authority, 4 validation, then extras resolved earlier (`account.rs:41-47`; `execute.rs:202-222`) |
| `AccountData { account_index, data_index, length }` | `4, ai, di, len` | A data slice of an earlier account. For example, a token account's owner sits at bytes 32..64, as used in `program-examples/…/account-data-as-seed/anchor/programs/transfer-hook/src/lib.rs:134-136`. (`account.rs:48-64`) |

Resolution runs in order, so a meta may reference only accounts before it
(`execute.rs:224-238`, where each resolved account is pushed to `account_key_data_refs`).

**Off-chain resolvers:**

| Client | Function | Behavior |
| --- | --- | --- |
| Rust | `spl_transfer_hook_interface::offchain::add_extra_account_metas_for_execute` | Fetches the validation PDA (an error if it is missing) and checks that source, mint, destination and authority are in the instruction. Builds a synthetic `Execute`, resolves each meta, then appends the resolved extras, the **hook program id** and the **validation PDA** to *your* instruction. (`transfer-hook/interface/src/offchain.rs:48-113`; `ExtraAccountMetaList::add_to_instruction`, `state.rs:262-307`) |
| JS legacy (`@solana/spl-token`) | `addExtraAccountMetasForExecute`, `createTransferCheckedWithTransferHookInstruction` | Same flow. A **no-op if the validation PDA does not exist.** (`clients/js-legacy/src/extensions/transferHook/instructions.ts:189-246`, `:261`) |
| JS kit (`@solana-program/token-2022`) | `resolveExtraAccountMetasForExecute` | Returns the extra metas plus the program id and validation PDA. Returns `[]` if no PDA exists. (`clients/js/src/transferHookExtraAccountMetas.ts:663-725`; per-meta `resolveExtraAccountMeta`, `:554-588`) |
| On-chain CPI helper (for programs that call `transfer_checked`) | `add_extra_accounts_for_execute_cpi` | Appends the extras, then the validation PDA, then the program id, to a `transfer_checked` CPI. This is what Orca uses. (`transfer-hook/interface/src/onchain.rs:67-135`; `orca…/util/v2/token.rs:70-86`) |

### A.3 Privilege de-escalation

| Account | Flags in the CPI to the hook | Source |
| --- | --- | --- |
| source, mint, destination, authority | **read-only, non-signer**. The authority's signature is never forwarded. | `pinocchio…/execute.rs:148-152` (`InstructionAccount::readonly`) |
| validation PDA | read-only, non-signer | `execute.rs:160` |
| extras | `is_signer` **always forced false**. `is_writable` is set to false if the same pubkey already appears read-only in the CPI, so a meta that resolves to source, mint, destination or authority can never be writable. Otherwise the declared `is_writable` is kept. | `execute.rs:267-289` (`de_escalate_account_meta`), applied at `:230`; identical logic in `spl-tlv-account-resolution/src/state.rs:33-53` |

Additional rules:

- **Writable extras must be writable in the outer transaction.** The runtime rejects writable or
  signer escalation relative to the caller (`agave/program-runtime/src/invoke_context.rs:363-376`,
  `PrivilegeEscalation`). In practice, an extra account (for example our policy-state PDA) is
  writable in the hook only if it is declared `is_writable` in the list and the client marks it
  writable in the outer transaction. The resolvers do this from the meta.
- **Only the owner can write.** Writes happen only to accounts the hook program owns. Its own
  PDAs are the normal case.
- **The reference hook writes to a writable extra.** It is a counter PDA in
  `program-examples/tokens/token-2022/transfer-hook/counter/anchor/programs/transfer-hook/src/lib.rs:122-131`
  (declared `is_writable = true`).
- **The JS kit resolver also de-escalates signers.** It strips signer *and* writable flags unless
  the same address is already signer or writable in the base metas, and the base metas are all
  read-only (`transferHookExtraAccountMetas.ts:606-627`, `:689-708`).

**Design consequence.** A hook can never act *as* the user. It cannot move the user's tokens and
cannot use the user's signature. Signed actions must happen in the top-level instruction or in a
program the user called directly.

### A.4 The `transferring` flag

- **What it is.**
  - `TransferHookAccount { transferring: Bool }` is a 1-byte account extension
    (`token-2022/interface/src/extension/transfer_hook/mod.rs:39-48`).
  - Every token account of a TransferHook mint is required to carry it
    (`interface/src/extension/mod.rs:1303`, `required_init_account_extensions`).
- **Lifecycle.**
  - Token-2022 sets `transferring = true` on **both** source and destination immediately before
    the CPI, after balances are already debited and credited (`processor.rs:570-580`, `:611-615`).
  - It clears both after the CPI returns (`:631-633`; `program/src/extension/mod.rs:64-70`).
  - The confidential-transfer path does the same (`confidential_transfer/processor.rs:808-839`).
- **What it proves.** Nobody but Token-2022 can write a Token-2022 account. If a hook sees
  `transferring == true` on source and destination, it is running inside a real Token-2022 transfer
  of those accounts. This defeats direct top-level calls to `Execute` with forged accounts.
- **How to check.** Unpack each account with `StateWithExtensions::<Account>`, read the
  `TransferHookAccount` extension, and require `transferring` on both source and destination. The
  error is `ProgramCalledOutsideOfTransfer` (`transfer-hook/program/src/processor.rs:25-34`,
  `:50-52`; Anchor version `program-examples/…/counter/…/lib.rs:82-93`).
- **What else to check.**
  - Owner == Token-2022 (`unpack` validates the layout; also check `owner`).
  - `source.mint == mint == destination.mint`.
  - The validation PDA address (`processor.rs:56-59`).
  - `check_account_infos` for the extras (`processor.rs:63-68`).

### A.5 Which instructions invoke the hook

| Token-2022 instruction | Hook invoked? | Source |
| --- | --- | --- |
| `TransferChecked` | **Yes** | `processor.rs:2110-2121` → `process_transfer(Checked)` → `:611-629` |
| `TransferCheckedWithFee` | **Yes** | `program/src/extension/transfer_fee/processor.rs:327-338` → `process_transfer(CheckedWithFee)` |
| `Transfer` (unchecked, deprecated) | **Fails** with `MintRequiredForTransfer` for hook-mint accounts, because they carry `TransferHookAccount` | `processor.rs:2043-2051`, `:440-447` |
| Confidential `Transfer` | Yes, with `amount = u64::MAX` | `confidential_transfer/processor.rs:656`, `:811-835` |
| **Self-transfer** (source == destination) | **No.** It returns `Ok` before the hook block. | `processor.rs:473`, `:537-542` |
| Permanent-delegate transfer | Yes, through the same path, with `authority = delegate` | `processor.rs:486-494` |
| `MintTo` / `MintToChecked` | **No** | `process_mint_to` (`processor.rs:1063-1155`) contains no hook or `Execute` reference. The only `Execute{…}.invoke()` sites are `processor.rs:620` and `confidential_transfer/processor.rs:826`. |
| `Burn` / `BurnChecked` | **No** | `process_burn` (`processor.rs:1156-…`), same evidence |
| Withdraw / harvest withheld fees | No | `transfer_fee/processor.rs:117-260`. There is no `Execute` site. |
| After-transfer callback | **None exists.** The hook *is* the post-balance-update callback: it runs after the balances are written and before `Ok`. A hook error reverts the whole transfer. There is no pre-transfer hook. | `processor.rs:570-638`; `transfer-hook/interface/src/lib.rs:1-4` |

**For the wrapper:**

- Wrap (vault deposit plus `mint_to`) and unwrap (`burn` plus vault release) **never run the
  hook**. Policies that must cover wrap and unwrap have to be enforced inside the wrapper program.
- Self-transfers skip the hook entirely.

### A.6 Adding or changing a transfer hook

- **SPL Token (Tokenkeg) mints cannot gain a hook.**
  - Token-2022 rejects any account not owned by Token-2022 (`check_program_account`,
    `interface/src/lib.rs:32-37`, called at `processor.rs:377-378`, `:403`).
  - SPL Token v1 has no extension TLV.
  - This is why we need a wrapper.
- **An already-initialized Token-2022 mint cannot gain one either.**
  - `TransferHook::Initialize` unpacks the mint with `unpack_uninitialized`
    (`transfer_hook/processor.rs:35-38`).
  - `unpack_uninitialized` fails with `AlreadyInUse` once the base mint is initialized
    (`interface/src/extension/mod.rs:862-874`).
  - `InitializeMint` checks the account length against the pre-written extension set
    (`processor.rs:132-137`).
  - `Reallocate` accepts only **Account**-type extensions (`program/src/extension/reallocate.rs:51-57`).
- **Init rules.**
  - At least one of `authority` and `program_id` must be set.
  - `program_id` must not be Token-2022 itself (`transfer_hook/processor.rs:41-49`).
- **The program id can be changed later.**
  - `TransferHookInstruction::Update` (accounts: mint, then authority plus multisig signers)
    lets `TransferHook.authority` set a new `program_id` (`transfer_hook/processor.rs:53-86`). It
    validates the authority with `validate_owner`, so a multisig works.
  - The authority can be rotated or removed with `SetAuthority(AuthorityType::TransferHookProgramId)`
    (`processor.rs:947-955`; `interface/src/instruction.rs:1263`, `:1293`).
  - Setting the authority to `None` freezes the hook program id permanently. **(inferred)** from
    `NoAuthorityExists` at `transfer_hook/processor.rs:67-68`.
- **Extra-account list updates** are a *hook-program* instruction, gated in the reference hook by
  the **mint authority** (`transfer-hook/program/src/processor.rs:144-200`). Each hook defines its
  own rule.

**Design.** The wrapper must create the Token-2022 mint itself, with `TransferHook` initialized
before `InitializeMint`.

- `TransferHook.authority` should be a wrapper-governed PDA, or `None` for an immutable hook.
- The wrapper can stay mint authority, so it is the only minter. That authority is also what the
  reference hook uses to gate `Initialize/UpdateExtraAccountMetaList`.

### A.7 AMM compatibility with TransferHook mints

| AMM | Policy | Do hook transfers work? | Source |
| --- | --- | --- | --- |
| **Raydium CP-Swap (CPMM)** | Allowlist: TransferFeeConfig, MetadataPointer, TokenMetadata, InterestBearingConfig, ScaledUiAmount. Anything else needs an admin `SupportMintAssociated` PDA `[SUPPORT_MINT_SEED, mint]` passed in remaining accounts. | **No (inferred).** `transfer_checked` CPIs pass only from, to, authority and mint, with no hook program or extra accounts, so a mint with an active hook program would fail with `MissingAccount`. | `programs/cp-swap/src/utils/token.rs:335-359`, `:305-333`; CPIs `:17-68`; gate at `instructions/initialize.rs:196-197` |
| **Meteora DAMM v2** | Permissionless: TransferFeeConfig, MetadataPointer, TokenMetadata. `TransferHook` passes only if **both** `program_id` and `authority` are `None`, meaning an inert hook. Otherwise an operator `TokenBadge` is required. | **No (inferred).** `transfer_from_user` and `transfer_from_pool` build `transfer_checked` with exactly 4 account infos. | `programs/cp-amm/src/utils/token.rs:222-289`; CPIs `:157-219` |
| **Orca Whirlpools** | `TransferHook` (also PermanentDelegate, MintCloseAuthority, DefaultAccountState, Pausable) is accepted **only with a `TokenBadge`** for that `WhirlpoolsConfig` and mint. | **Yes.** The `_v2` instructions take `transfer_hook_accounts` from remaining accounts (slices `TransferHookA`, `TransferHookB`, `TransferHookReward`) and call `add_extra_accounts_for_execute_cpi`. They return `NoExtraAccountsForTransferHook` if none are given. | `programs/whirlpool/src/util/v2/token.rs:20-91`, `:150-155`, `:208-300`, `:310-322`; `util/v2/remaining_accounts_utils.rs:8-10` |

**Consequences:**

- Today a hooked wrapped asset trades only on venues that forward hook accounts. Of these three,
  that is only Orca, and only behind a per-config badge.
- On Orca's vault-to-owner leg, the hook's `authority` is the Whirlpool PDA, not the user.
- Pool vaults are ordinary token accounts. Hooks must not assume `authority` is a person.

### A.8 Compute and CPI depth

- **Depth limits.**
  - The maximum instruction stack depth is **5**, which is 4 nested CPIs.
  - SIMD-0268 raises it to 9 (`agave/program-runtime/src/execution_budget.rs:8-18`). It is not
    active on mainnet (see `docs/research.md` §2).
- **Where the hook runs:**

  | Caller | Hook stack height | Further CPIs the hook can make |
  | --- | --- | --- |
  | User calls `transfer_checked` top-level | 2 (Token-2022 is 1) | 3 |
  | User calls program P, which CPIs `transfer_checked` | 3 | 2 |
  | Aggregator → AMM → Token-2022 | 4 | 1 |
  | 4-deep caller | Exceeds the limit; the transaction fails | — |

  **(inferred)** from the depth constant.
- **Re-entrancy.**
  - A→B→A is forbidden. Only direct self-recursion is allowed
    (`agave/program-runtime/src/invoke_context.rs:246-270`, `ReentrancyNotAllowed`).
  - A hook therefore **cannot CPI Token-2022**, which is on the stack and not last. It cannot
    move tokens of any mint during a hook.
  - A hook also cannot CPI back into any program above it. If our wrapper or smart account is the
    caller of `transfer_checked`, the hook cannot call into the wrapper or smart account. Shared
    state must be passed as read-only accounts, or owned by the hook.
- **Cost.**
  - Each CPI costs 946 CU (`execution_budget.rs:21`) plus account serialization.
  - The `Execute` CPI includes 5 + M accounts. On every transfer, Token-2022 re-derives the
    validation PDA (`Address::derive_program_address`, `pinocchio…/src/lib.rs:21-32`, called at
    `execute.rs:139-140`) and every PDA extra (`find_program_address`, `account.rs:67`).
  - Keep M small and prefer literal metas over PDA metas where possible.

---

## PART B — Smart accounts and delegated authority

### B.1 Squads v4 (`SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`, `lib.rs:36`)

**PDAs** (`state/seeds.rs:1-10`):

| Account | Seeds | Source |
| --- | --- | --- |
| Multisig | `["multisig", "multisig", create_key]` | `instructions/vault_transaction_execute.rs:10-11` |
| Vault *i* | `["multisig", multisig, "vault", vault_index: u8]`. The vault holds no data; it is a PDA signer only. | `vault_transaction_execute.rs:101-107` |
| Transaction | `["multisig", multisig, "transaction", index: u64 LE]` | `:30-38` |
| Proposal | `[…, "transaction", index, "proposal"]` | `:16-26` |
| Ephemeral signer | `["multisig", transaction_key, "ephemeral_signer", idx: u8]` | `utils/ephemeral_signers.rs:15-21` |
| Spending limit | `["multisig", multisig, "spending_limit", create_key]` | `instructions/spending_limit_use.rs:31-40` |

**Multisig state** (`state/multisig.rs:12-41`):

- `config_authority`: `default` means the multisig is autonomous. Any other key makes it a
  "controlled multisig", where that key alone changes the config.
- `threshold: u16`.
- `time_lock: u32` seconds, capped at 3 months (`MAX_TIME_LOCK`, `:9`, `:189-192`).
- `transaction_index` and `stale_transaction_index`.
- `members: Vec<Member{key, permissions}>`, sorted, with permission bits Initiate=1, Vote=2,
  Execute=4 (`:253-262`).
- `invariant()` checks:
  - no duplicate members;
  - at least one proposer, voter and executor;
  - `0 < threshold ≤ voters`;
  - `stale ≤ tx_index` (`:138-195`).
- A config change calls `invalidate_prior_transactions` (`stale = tx_index`, `:200-202`).

**Flow:**

1. `vault_transaction_create` (needs Initiate) stores a `VaultTransaction` with `vault_index`,
   `ephemeral_signer_bumps` and a compiled `VaultTransactionMessage` (`instructions/vault_transaction_create.rs:52-67`, `:90-119`).
2. `proposal_create` / `activate`.
3. `proposal_vote` (needs Vote):
   - Approve or Reject requires the proposal to be `Active` and **not stale**
     (`transaction_index > stale_transaction_index`). Cancel requires `Approved` and is allowed
     even when stale (`instructions/proposal_vote.rs:55-85`).
   - The proposal becomes `Approved` when `approved.len() ≥ threshold` and `Rejected` when
     rejections reach `cutoff = voters − threshold + 1` (`state/proposal.rs:44-100`;
     `multisig.rs:219-228`).
4. `vault_transaction_execute` (needs Execute) requires `Approved{timestamp}` and
   `now − timestamp ≥ time_lock` (`vault_transaction_execute.rs:57-76`). An approved proposal that
   later went stale can still execute (`:77-78`).

**Vault execution of arbitrary CPIs** (`utils/executable_transaction_message.rs`):

- **`new_validated` defeats account substitution:**
  - The lookup-table account count must equal the message's lookups. Each table must be owned by
    the ALT program and appear at the same index (`:39-65`).
  - `message_account_infos.len()` must equal `num_all_account_keys` (`:67-72`).
  - Each static key must match by pubkey. Signer flags must match, except for the vault and
    ephemeral PDAs, and writable flags must match (`:76-98`).
  - ALT-loaded keys must match the on-chain table contents, and writable ones must be writable
    (`:117-163`).
- **`execute_message`:**
  - It `invoke_signed`s every instruction with the vault seeds plus all ephemeral-signer seeds
    (`:180-213`).
  - **There is no program allowlist: the vault signs for whatever program the approved message
    names.**
  - Signer flags in each CPI come from the message's header, not from the account infos
    (`:266-274`).
- **Re-entrancy guard.** `protected_accounts = [proposal]`. Any CPI that passes a protected
  account as writable fails with `ProtectedAccount` (`:204-210`; `vault_transaction_execute.rs:134`;
  batch: `[proposal, batch]`, `batch_execute_transaction.rs:154`).
- **Consume before execute.** `transaction.take()` is called before execution, so the message is
  consumed (`vault_transaction_execute.rs:91-96`). The status becomes `Executed` afterwards
  (`:149-151`).

**SpendingLimit** (`state/spending_limit.rs:5-51`):

- Fields:
  - `{multisig, create_key, vault_index, mint (default = SOL), amount, period, remaining_amount, last_reset, members, destinations}`.
  - `members` are arbitrary signers, **independent of multisig membership** (`:40-46`).
  - An empty `destinations` list means any destination.
  - `Period` is OneTime, Day, Week or Month, where a month is 30 days (`:86-107`).
- `spending_limit_use` (`instructions/spending_limit_use.rs`):
  1. Requires the signer to be in `members`, the mint to match, and the destination to be in the
     allowlist when one is set (`:90-135`).
  2. Resets per elapsed periods. `last_reset` is advanced by whole periods (`:148-163`).
  3. Applies `checked_sub` of `remaining_amount`, or fails with `SpendingLimitExceeded` (`:165-170`).
  4. Makes **a single direct `transfer_checked` or system transfer, signed by the vault PDA**
     (`:173-254`).
- This is an *authorized direct transfer*, not a filter on arbitrary transactions. **(inferred)**
  Its `transfer_checked` CPI forwards no hook accounts (`:234-253`), so v4 spending limits cannot
  move Token-2022 transfer-hook mints.

**Rent reclaim and close.** `transaction_accounts_close` sends lamports only to
`multisig.rent_collector` (an address constraint) and only for terminal or stale states. Approved
vault transactions that went stale cannot be closed (`instructions/transaction_accounts_close.rs:1-10`,
`:23-56`, `:72-108`). Multisig `realloc_if_needed` checks `owner == program` before resizing
(`state/multisig.rs:82-134`).

### B.2 Squads smart-account program (`SMRTzfY6DfH5ik3TKiyLFfXexV8uSG3d2UksSCYdunG`, `lib.rs:40`)

This is v4's successor and the on-chain base Squads uses for programmable accounts. No separate
"Grid" source was found. The README has no "grid" mention, so we do not cite Grid.

**PDAs** (`state/seeds.rs`):

| Account | Seeds |
| --- | --- |
| Settings | `["smart_account", "settings", seed: u128 LE]`. The seed is a counter in `ProgramConfig`, not user input (`state/settings.rs:22-29`; `seeds.rs:39-45`). |
| Smart account *i* | `["smart_account", settings, "smart_account", account_index: u8]` (`seeds.rs:57-67`) |
| Policy | `["smart_account", "policy", settings, policy_seed: u64]` (`seeds.rs:47-54`) |
| Hook authority | `create_program_address(["hook_authority_seeds"])`, **with no bump**: one global off-curve key, `2MTRji19…DaHj8` (`seeds.rs:15-30`, `:78-81`) |

**Settings** (`state/settings.rs:21-67`):

- Fields mirror v4: `settings_authority`, `threshold`, `time_lock` (≤ 3 months), transaction and
  stale indexes, `signers` with Initiate, Vote and Execute.
- It adds `archival_authority`, `account_utilization` and `policy_seed`.
- `generate_core_state_hash()` hashes signers, permissions, threshold and time lock (`:71-90`).

**Policies.** Each `Policy` account is a separate consensus account with its own signers,
threshold, time lock, `start`, and `expiration` (`state/policies/policy_core/policy.rs:31-67`).

- Expiration is `Timestamp(i64)`, or `SettingsState(hash)`: the policy dies automatically as soon as
  the parent settings' core hash changes (`:15-21`, `:340-375`).
- `PolicyState` is one of `InternalFundTransfer`, `SpendingLimit`, `SettingsChange` or
  `ProgramInteraction` (`:226-235`).

**`ProgramInteractionPolicy`** (`state/policies/implementations/program_interaction.rs`). This is
the closest prior art to our hook stacks.

- **Structure.** `account_index`, `instructions_constraints` (OR across constraints),
  `pre_hook` / `post_hook: Option<Hook>`, and `spending_limits: Vec<SpendingLimitV2>`
  (`:23-35`).
- **`InstructionConstraint`:**
  - It pins `program_id` and holds AND-ed `AccountConstraint`s and `DataConstraint`s (`:37-45`).
  - `DataConstraint` = `{data_offset, DataValue (u8…u128 LE | bytes), operator (=, ≠, >, ≥, <, ≤)}`.
  - `AccountConstraint` = `{account_index, Pubkey(list) | AccountData(constraints), owner}`
    (`:92-159`; evaluation `:186-341`).
- **Limits.** At most 20 constraints and 10 spending limits (`:611-618`).
- **The payload says which constraint applies.** The payload supplies one constraint index per
  instruction. Count and bounds are checked (`:858-879`), and constraints are evaluated before
  execution (`:496-535`).
- **Hooks never get the vault's signature.** `Hook::execute` (`:412-494`) works as follows:
  - It CPIs `hook.program_id` with `hook.instruction_data`, optionally plus the serialized inner
    instructions and all their accounts (`pass_inner_instructions`).
  - It signs **only with `HOOK_AUTHORITY`** (`invoke_signed(…, &[&[SEED_HOOK_AUTHORITY]])`,
    `:487-492`).
  - The hook authority is marked read-only signer (`:437-444`, `:466-475`).
  - Hook programs authenticate the caller by requiring this signer.
  - The smart-account PDA reaches the hook with its *outer* `is_signer`, which is false for a PDA
    (`:440-443`).
- **Order of execution** (async path `:906-1021`; sync path `:1028-1131`):
  1. validate the message;
  2. evaluate the constraints;
  3. run the **pre-hook**;
  4. reset limits and snapshot balances;
  5. execute the CPIs;
  6. check the balance deltas against the limits;
  7. run the **post-hook**.
- **Spending limits by balance delta, not by parsing instructions.**
  - `check_pre_balances` records the smart account's lamports and every **writable**
    token-program-owned account whose owner is the smart account (`utils/account_tracking.rs:26-79`).
  - After execution, `evaluate_balance_changes` (`:82-194`):
    - debits per mint must be ≤ `remaining_in_period`, and are then decremented;
    - **untracked mints must not decrease at all**;
    - **closing a tracked token account is an error**;
    - **the delegate must be unchanged**, and the delegated amount may only decrease;
    - **the owner must be unchanged**.
  - This blocks the approve, set-authority and close side channels.
- **`SpendingLimitV2`** fields (`state/policies/utils/spending_limit_v2.rs:5-60`):
  - `PeriodV2`: OneTime, Daily, Weekly, Monthly or `Custom(secs)`.
  - `TimeConstraints{start, expiration, period, accumulate_unused}`.
  - `QuantityConstraints{max_per_period, max_per_use, enforce_exact_quantity}`.

**Synchronous execution** (`instructions/transaction_execute_sync.rs`;
`utils/context_validation.rs:4-64`):

- This is a one-transaction mode with no proposal account.
- It requires `time_lock == 0` and signers ≥ threshold, with no duplicates, all real signers.
- The signers' **aggregated permissions must equal Initiate|Vote|Execute**.
- `SynchronousTransactionMessage::new_validated` (`utils/synchronous_transaction_message.rs:16-67`)
  changes privileges before the CPIs:
  - it sets the smart-account PDA to signer;
  - it **forces the settings account non-writable** ("prevents dangerous re-entrancy", `:51-53`);
  - it **strips the signer bit from consensus signers** (`:54-58`).
- Each CPI is checked by `LogEvent::check_instruction`, which forbids CPIs into the program's own
  `log_event` instruction, so events cannot be spoofed (`:95-96`; `instructions/log_event.rs:84-96`).
- The async path keeps v4's `protected_accounts = [proposal]` (`instructions/transaction_execute.rs:167`).

### B.3 Session keys

**MagicBlock `gpl_session`** (`KeyspM2ssCJbqUhQ4k7sveSiY4WjnYsrXkC8oDbwde5`,
`programs/gpl_session/src/lib.rs:12`):

| Aspect | Behavior | Source |
| --- | --- | --- |
| Token PDA | `["session_token" \| "session_token_v2", target_program, session_signer, authority]` | `lib.rs:82-93`, `:272-283`, `:452`, `:484` |
| Data | `{authority, target_program, session_signer, valid_until: i64}`. V2 adds `fee_payer`. | `:430-448` |
| Creation | `authority` and `session_signer` both sign, and `target_program` must be executable. `valid_until` defaults to now + 1 h and may be at most now + 7 days. There is an optional lamport top-up to the session key. | `:72-77`, `:95-104`, `:130-150` |
| Scope | **Only `target_program`.** There are no instruction, amount or account scopes. The target program decides what a session may do. | `:432-437`; the macro hardcodes `target_program() = crate::id()` (`macros/attribute/src/lib.rs:155-158`) |
| Validation | The consuming program derives `Session` and wraps handlers in `#[session_auth_or(expr, err)]`. If a token is present, it re-derives the PDA from (`crate::id()`, signer, authority), checks expiry, and checks `token.authority == session_authority`. Otherwise it falls back to `expr`. | `lib.rs:460-479`, `:495-515`, `:518-554`; `macros/attribute/src/lib.rs:228-268` |
| Revocation | V1: **anyone** may revoke, and rent goes to the authority. V2: anyone may revoke once expired; while active, the authority must sign. Rent goes to `fee_payer`. | `lib.rs:231-265`, `:373-413` |
| Pitfall | V1 `is_expired()` returns `now < valid_until`, which is true when **valid**. The name is inverted, but `validate` returns it as "valid". V2 fixes the naming. | `:454-457`, `:487-492` |

**Swig session authorities** (see B.4) keep the session *inside* the role:

- `Ed25519SessionAuthority{public_key, session_key, max_session_length, current_session_expiration}`.
- Durations are in **slots** (`state/src/authority/ed25519.rs:169-178`).
- `start_session` rejects `duration > max_session_length` and sets `expiration = slot + duration`
  (`:255-265`).
- `authenticate_session` rejects `slot > expiration`, then requires the `session_key` account to
  be a signer (`:268-286`, `:316-328`).
- The same variants exist for secp256k1, secp256r1 and ProgramExec (`authority/mod.rs:119-155`).
- Sessions therefore inherit the role's full action list.

### B.4 Swig (`swigypWHEksbC64pWKwah1WTeh9JXwx8H1rJHLdbQMB`, `program/src/lib.rs:38`)

**Accounts:**

- Config: `["swig", id]` (`state/src/swig.rs:28-36`).
- Wallet address: `["swig-wallet-address", swig_key]`, a system-owned PDA that holds funds and
  signs (`swig.rs:49-60`).
- The config holds a list of **roles**.

**Role layout.**

- `Position{authority_type: u16, authority_length: u16, num_actions: u16, id: u32, boundary: u32}`,
  followed by the authority bytes, followed by the actions (`state/src/role.rs:20-47`).
- Each action is an `Action{action_type: u16, length: u16, boundary: u32}` header plus fixed
  data (`state/src/action/mod.rs:65-74`).
- Boundaries must chain exactly (`:428-432`).
- Non-repeatable actions may appear only once (`:351-404`).

**Authority types** (`state/src/authority/mod.rs:119-155`):

- Ed25519, Secp256k1 and Secp256r1 (passkeys), each with a Session variant.
- **ProgramExec.** The role is authorized because a **preceding instruction in the same
  transaction**, read from the instructions sysvar, targeted a configured `program_id` with a
  configured data prefix of up to 40 bytes (`authority/programexec/mod.rs:1-53`, `:170-204`).

**Permissions** (`Permission` enum, `action/mod.rs:127-192`):

| Group | Actions |
| --- | --- |
| Root | `All`, `AllButManageAuthority`, `ManageAuthority`, `ReplaceAuthority`, `CloseSwigAuthority` |
| Program scope | `Program` (allowlist one program id, repeatable), `ProgramAll`, `ProgramCurated`, `ProgramScope` (tracks a balance field range in a program-owned account) |
| SOL | `SolLimit`, `SolRecurringLimit`, `SolDestinationLimit`, `SolRecurringDestinationLimit` |
| Tokens | `TokenLimit`, `TokenRecurringLimit`, `TokenDestinationLimit`, `TokenRecurringDestinationLimit` |
| Stake | `StakeLimit`, `StakeRecurringLimit`, `StakeAll` |
| Sub-accounts | `SubAccount` (v1); `SubAccountV2Create/All/Sign/Withdraw/Toggle`, scoped per `subacc_id` |

**Execution: `sign_v2`** (`program/src/actions/sign_v2.rs:235-…`):

1. **Classify accounts.** Account 0 must be this Swig config and account 1 the wallet address
   (`:243-256`; classifier `program/src/lib.rs:341-380`).
2. **Authenticate the role.** The role is found by `role_id`. A session-based authority uses
   `authenticate_session`; otherwise `authenticate` (`:267-286`).
3. **`All` / `AllButManageAuthority`.** Snapshot every writable, unclassified account. Execute each
   inner instruction, rejecting changes to the wallet address's shape. Then
   `isolation.validate()` (`:302-340`).
4. **Otherwise, snapshot before any CPI.**
   - Every writable account is hashed with **only its permitted balance bytes excluded**:
     - the Swig config is hashed fully, with its owner;
     - token accounts exclude only amount bytes 64..72, with WSOL and transfer-fee handling;
     - stake accounts exclude the delegation and state fields;
     - `ProgramScope` accounts exclude the configured range.
   - Other writable accounts are snapshotted fully (`:346-459`).
5. **Per inner instruction, check the program.** If the instruction uses the Swig signer, the
   program must be allowed by `ProgramAll`, by `ProgramCurated`, or by a matching `Program` action,
   else `PermissionDeniedMissingPermission` (`:461-475`).
6. **After each CPI, record debits.** It accumulates lamport and token debits, and treats a closed
   token account as fully spent (`:492-623`).
7. **Final checks.**
   - The Swig config must be unchanged by hash (`:630-650`).
   - The wallet must still be rent-exempt (`:652-657`).
   - Total spend is charged against the limit actions (`:659-…`).

Signer bits:

- The CPI signer bit is set for the wallet PDA.
- Outer signers are forwarded, with no restricted keys in `SignV2` (`:287-297`;
  `instructions/src/lib.rs:297-310`).
- The inner instruction tells Swig whether it uses the wallet signer (`uses_swig_signer`). Only
  those instructions need a program permission.

Audits are shipped in-repo: `audits/2025-accretion-…pdf`, `audits/2026-halborn-…pdf`. Squads
audits are in `Squads-Protocol_v4/audits/` and `Squads-Protocol_smart-account-program/audits/`.

### B.5 Attack classes and the mitigations in these codebases

| Attack | Mechanism | Mitigation seen (cite) | Our requirement |
| --- | --- | --- | --- |
| **Account substitution** | The caller passes a different account than the approved message intended. | v4 / smart account: exact count, pubkey, signer and writable match against the stored message, and ALT owner and contents checks (`v4/…/executable_transaction_message.rs:39-163`). Anchor `seeds` / `has_one` on every PDA (`vault_transaction_execute.rs:9-39`). Token hooks: `check_account_infos` checks exact positions (`tlv state.rs:209-259`). | Store the account list in the approved intent, or derive it. Re-verify keys and flags at execution. |
| **Signer-privilege leakage through `invoke_signed` into arbitrary programs** | The vault PDA signature is handed to an unapproved program, which can then approve, set authority, or drain. | v4: only member-approved messages are signed. Swig: per-program allowlist whenever the wallet signer is used (`sign_v2.rs:461-475`). Smart account: hooks receive a **separate** `HOOK_AUTHORITY` signer, never the vault (`program_interaction.rs:437-492`). Sync mode strips consensus signers' signer bit (`synchronous_transaction_message.rs:54-58`). Token-2022 never forwards the authority's signature to hooks (`execute.rs:148-152`, `:288`). | Never pass the account PDA as signer to a hook. Give each hook program a distinct, no-power signer (see `docs/research.md` §5.5). Allowlist the CPI targets that the vault signs for. |
| **Side-channel value extraction** (approve, set-authority, close, realloc on vault token accounts) | An approved CPI moves no balance directly but grants a delegate, changes the owner, or closes an account. | Smart account: post-execution delegate, owner and close checks (`account_tracking.rs:122-191`). Swig: full-account hashes, excluding only balance bytes, so delegate or owner edits fail; a close is charged as a full spend (`sign_v2.rs:380-412`, `:523-529`). | Hash writable vault-owned accounts before and after, allowing only balance bytes to change. Count closes as full spends. |
| **CPI re-entry into the wallet program** (self-CPI is allowed: A→A, `invoke_context.rs:246-270`) | The executed message calls the wallet program to mutate the proposal or settings mid-execution. | v4 / smart account: `protected_accounts = [proposal(, batch)]` must not be writable in any CPI (`executable_transaction_message.rs:204-210`). Settings forced read-only in sync mode (`synchronous_transaction_message.rs:51-53`). The message is `take()`n before execution (`vault_transaction_execute.rs:91-96`). `log_event` self-CPI is blocked (`log_event.rs:84-96`). Swig: config account hash compared after the CPIs (`sign_v2.rs:630-650`). | Mark the intent consumed before the CPIs. Forbid our own state accounts as writable in user CPIs. Hash-check config after execution. |
| **Stale approvals after a config change** | A tx approved under old signers or threshold executes after the membership changed. | `stale_transaction_index`: a stale tx cannot be voted (`proposal_vote.rs:69-74`). But v4 *does* let a stale-but-approved tx execute (`vault_transaction_execute.rs:77-78`). Smart account adds `PolicyExpiration::SettingsState(hash)` (`policy.rs:355-375`). | Invalidate pending intents and session grants on any guardian, threshold or timelock change, including approved-but-unexecuted ones (stricter than v4). |
| **Close / rent theft** | Closing transaction or proposal accounts to an attacker, or closing early to cancel a timelock. | v4: `rent_collector` address constraint, close allowed only for terminal or stale states (`transaction_accounts_close.rs:23-108`). Session keys V2: authority must sign while active (`gpl_session/src/lib.rs:404-413`). | Close only to the recorded rent payer, and only in terminal states. |
| **Realloc** | Growing or shrinking state so that layout or rent is corrupted. | v4 `realloc_if_needed` checks the owner and tops up rent (`multisig.rs:82-134`). Swig rejects wallet-address shape mutation (`sign_v2.rs:320-334`, `:477-490`). | Do realloc only in the owning program, with an explicit max size. |
| **Unauthenticated hook entry** (Cork class, see `docs/research.md` §1.7) | A hook is called directly with forged context. | Token hooks: the `transferring` flag (A.4). Smart account: the `HOOK_AUTHORITY` signer. | Every hook checks a caller-proof first. |
| **Hook bypass on mint/burn and self-transfer** | Value moves without the hook. | The protocol behaves this way by design (A.5). | The wrapper enforces its own policy at wrap and unwrap. Hooks must not assume they see all supply movement. |

---

## Design constraints (summary)

1. **Hook interface is fixed.**
   - Data is `disc(8) = 69 25 65 c5 4b fb 66 1a ‖ amount u64`.
   - Accounts are `[source, mint, destination, authority, extra_metas_pda, extras…]`.
   - All base accounts are read-only and non-signer. Extras are never signers.
   - Extras are writable only if declared, not equal to a base account, and writable in the outer
     transaction.
2. **The validation PDA is per hook program.** Seeds are `["extra-account-metas", mint]`, so one
   mint has exactly one hook program and one list.
   - A *stack* of hooks behind one mint requires a single dispatcher hook program.
   - That program CPIs our sub-hooks, each costing a level of depth.
3. **The hook runs post-balance and can only veto.** It runs after the balances are written, can
   veto by returning an error, cannot call Token-2022, and cannot call any program above it.
4. **Wrapping is invisible to the hook.** Wrap and unwrap use `mint_to` / `burn`, so the hook never
   sees them. Self-transfers also skip the hook.
5. **The mint must be created by the wrapper** with `TransferHook` set before `InitializeMint`.
   Existing SPL mints cannot gain a hook. Hook program id changes require `TransferHook.authority`.
6. **DEX reach is limited.** Only Orca forwards hook accounts, and only with a TokenBadge. Raydium
   CPMM and Meteora DAMM v2 do not forward them **(inferred)**.
7. **What to reuse for smart accounts:**
   - v4 message validation, `protected_accounts`, the stale index and time lock.
   - The smart account's policy model: instruction/data/account constraints, pre/post hooks
     signed by a dedicated hook-authority PDA, balance-delta spending limits with
     delegate/owner/close checks, and settings-hash expiry.
   - Swig's role and action model, per-program allowlist when the wallet signs, full-account
     hashing that excludes only balance bytes, slot-bounded sessions, and ProgramExec authorities.
