//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_lib::prelude::*;

/// A single cross-component call to execute as part of a bounty.
///
/// Arguments are split into:
/// - `static_args`: pre-encoded (borsh) bytes known at registration time, one entry per arg.
/// - `append_executor`: when `true`, the executor's `ComponentAddress` is borsh-encoded and
///   appended as the final argument at execution time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub target: ComponentAddress,
    pub method: String,
    /// Pre-encoded borsh bytes, one per argument.  Use `vec![]` for no static args.
    pub static_args: Vec<Vec<u8>>,
    /// When `true`, `executor_account` is encoded and appended as the last argument.
    pub append_executor: bool,
}

/// Read-only summary returned by `Bounty::info()`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BountyInfo {
    pub active: bool,
    pub fee_per_run: Amount,
    pub budget_remaining: Amount,
    pub min_epoch: u64,
    pub interval_epochs: Option<u64>,
    pub last_executed_epoch: u64,
    pub executed_count: u64,
    pub next_eligible_epoch: u64,
    pub step_count: u64,
}

#[template]
mod bounty_template {
    use super::*;

    /// A deferred execution task — one-shot or recurring — backed by a fee vault.
    ///
    /// # One-shot mode (`interval_epochs = None`)
    /// The steps execute once when the settlement epoch is reached and the bounty becomes
    /// inactive.
    ///
    /// # Recurring mode (`interval_epochs = Some(N)`)
    /// The steps execute every N epochs.  The bounty stays active until the vault can no longer
    /// cover the next run, or until the owner cancels it.
    ///
    /// # Owner operations (protected by the owner badge via access rules)
    /// - `top_up(funds)` — add more budget; also reactivates an exhausted bounty.
    /// - `set_fee_per_run(amount)` — change the per-run fee.
    /// - `set_interval(epochs)` — switch between one-shot and recurring.
    /// - `cancel()` — deactivate and return remaining funds to `owner_account`.
    pub struct Bounty {
        owner_badge: NonFungibleAddress,
        owner_account: ComponentAddress,
        steps: Vec<TaskStep>,
        fee_vault: Vault,
        fee_per_run: Amount,
        min_epoch: u64,
        interval_epochs: Option<u64>,
        last_executed_epoch: u64,
        executed_count: u64,
        active: bool,
    }

    impl Bounty {
        /// Create a new bounty.
        ///
        /// * `owner_badge` — the owner's badge NFT address (e.g. account badge).
        /// * `owner_account` — account to receive remaining budget if cancelled.
        /// * `steps` — task steps to execute atomically on each run.
        /// * `fee` — initial budget bucket.
        /// * `fee_per_run` — fee paid to the executor per run (must be ≤ `fee.amount()`).
        /// * `min_epoch` — earliest epoch at which the bounty may execute (0 = immediately).
        /// * `interval_epochs` — `None` for one-shot; `Some(N)` to repeat every N epochs.
        pub fn new(
            owner_badge: NonFungibleAddress,
            owner_account: ComponentAddress,
            steps: Vec<TaskStep>,
            fee: Bucket,
            fee_per_run: Amount,
            min_epoch: u64,
            interval_epochs: Option<u64>,
        ) -> Component<Self> {
            assert!(!steps.is_empty(), "Bounty must have at least one task step");
            assert!(!fee_per_run.is_zero(), "fee_per_run must be greater than zero");
            assert!(
                fee.amount() >= fee_per_run,
                "Initial fee budget must cover at least one run"
            );

            let access_rules = ComponentAccessRules::new()
                .method("execute", rule!(allow_all))
                .method("is_eligible", rule!(allow_all))
                .method("is_active", rule!(allow_all))
                .method("info", rule!(allow_all))
                .method("top_up", rule!(non_fungible(owner_badge.clone())))
                .method("set_fee_per_run", rule!(non_fungible(owner_badge.clone())))
                .method("set_interval", rule!(non_fungible(owner_badge.clone())))
                .method("cancel", rule!(non_fungible(owner_badge.clone())));

            Component::new(Self {
                owner_badge,
                owner_account,
                steps,
                fee_vault: Vault::from_bucket(fee),
                fee_per_run,
                min_epoch,
                interval_epochs,
                last_executed_epoch: 0,
                executed_count: 0,
                active: true,
            })
            .with_access_rules(access_rules)
            .create()
        }

        /// Execute the bounty if it is eligible.
        ///
        /// Runs all task steps atomically.  Pays `fee_per_run` to `executor_account`.
        /// Deactivates if one-shot or vault can no longer cover the next run.
        pub fn execute(&mut self, executor_account: ComponentAddress) {
            assert!(self.active, "Bounty is not active");

            let current_epoch = Consensus::current_epoch();
            let next_eligible = self.next_eligible_epoch();
            assert!(
                current_epoch >= next_eligible,
                "Bounty is not yet eligible: current epoch {}, eligible from epoch {}",
                current_epoch,
                next_eligible,
            );
            assert!(
                self.fee_vault.balance() >= self.fee_per_run,
                "Insufficient fee budget to pay executor"
            );

            // Execute all steps atomically.  A panic in any step reverts the whole transaction.
            // Use `call::<_, tari_bor::Value, _>` so we can discard return values of any type.
            for step in &self.steps {
                let mut call_args: Vec<Vec<u8>> = step.static_args.clone();
                if step.append_executor {
                    call_args.push(tari_bor::encode(&executor_account).unwrap());
                }
                let _: tari_bor::Value = ComponentManager::get(step.target).call(&step.method, call_args);
            }

            // Pay the executor.
            let fee_bucket = self.fee_vault.withdraw(self.fee_per_run);
            ComponentManager::get(executor_account).invoke("deposit", args![fee_bucket]);

            self.last_executed_epoch = current_epoch;
            self.executed_count += 1;

            emit_event("BountyExecuted", metadata![
                "executor" => executor_account.to_string(),
                "epoch" => current_epoch.to_string(),
                "run_number" => self.executed_count.to_string(),
            ]);

            // Deactivate if one-shot, or if vault can't cover the next run.
            if self.interval_epochs.is_none() || self.fee_vault.balance() < self.fee_per_run {
                self.active = false;
            }
        }

        /// Returns `true` if the bounty can be executed in the current epoch.
        pub fn is_eligible(&self) -> bool {
            if !self.active {
                return false;
            }
            let current_epoch = Consensus::current_epoch();
            current_epoch >= self.next_eligible_epoch() && self.fee_vault.balance() >= self.fee_per_run
        }

        /// Returns `true` if the bounty is still active (not cancelled and not exhausted).
        pub fn is_active(&self) -> bool {
            self.active
        }

        /// Returns a summary of the bounty's current state.
        pub fn info(&self) -> BountyInfo {
            BountyInfo {
                active: self.active,
                fee_per_run: self.fee_per_run,
                budget_remaining: self.fee_vault.balance(),
                min_epoch: self.min_epoch,
                interval_epochs: self.interval_epochs,
                last_executed_epoch: self.last_executed_epoch,
                executed_count: self.executed_count,
                next_eligible_epoch: self.next_eligible_epoch(),
                step_count: self.steps.len() as u64,
            }
        }

        // ── Owner-only methods ──────────────────────────────────────────────────

        /// Add more budget.  Reactivates an exhausted bounty if balance now covers a run.
        /// Caller must present the owner badge.
        pub fn top_up(&mut self, funds: Bucket) {
            assert_eq!(
                funds.resource_address(),
                self.fee_vault.resource_address(),
                "Top-up must use the same resource as the fee vault"
            );
            self.fee_vault.deposit(funds);
            if !self.active && self.fee_vault.balance() >= self.fee_per_run {
                self.active = true;
            }
        }

        /// Change the per-run fee.  Caller must present the owner badge.
        pub fn set_fee_per_run(&mut self, new_fee: Amount) {
            assert!(!new_fee.is_zero(), "fee_per_run must be greater than zero");
            self.fee_per_run = new_fee;
        }

        /// Change the recurrence interval.  Pass `None` to make the bounty one-shot.
        /// Caller must present the owner badge.
        pub fn set_interval(&mut self, interval_epochs: Option<u64>) {
            self.interval_epochs = interval_epochs;
        }

        /// Cancel and return remaining budget to `owner_account`.
        /// Caller must present the owner badge.
        pub fn cancel(&mut self) {
            assert!(self.active, "Bounty is already cancelled");
            self.active = false;
            let remaining = self.fee_vault.withdraw_all();
            ComponentManager::get(self.owner_account).invoke("deposit", args![remaining]);
            emit_event("BountyCancelled", metadata!["owner" => self.owner_account.to_string()]);
        }

        // ── Private helpers ────────────────────────────────────────────────────

        fn next_eligible_epoch(&self) -> u64 {
            if self.last_executed_epoch == 0 {
                self.min_epoch
            } else {
                self.interval_epochs
                    .map(|i| self.last_executed_epoch + i)
                    .unwrap_or(u64::MAX)
            }
        }
    }
}
