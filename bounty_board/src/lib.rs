//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_lib::prelude::*;

/// Mirror of `bounty::TaskStep` — must stay binary-compatible (identical CBOR field layout).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub target: ComponentAddress,
    pub method: String,
    pub static_args: Vec<Vec<u8>>,
    pub append_executor: bool,
}

#[template]
mod bounty_board_template {
    use super::*;

    /// A global registry that indexes `Bounty` components so executor bots have a single
    /// well-known address to scan.
    ///
    /// The board holds no funds — each bounty's state and vault live in its own `Bounty`
    /// component created via `TemplateManager`.
    ///
    /// # Workflow
    /// 1. **Post** — call `post(...)`.  Creates a `Bounty` component and records its address.
    /// 2. **Scan** — call `list_open()` to get all registered bounty addresses, then call
    ///    `bounty.is_eligible()` on each to filter candidates.
    /// 3. **Execute** — call `bounty.execute(my_account)` directly on the bounty.
    /// 4. **Prune** — call `prune()` to remove inactive bounties from the list.
    pub struct BountyBoard {
        bounty_template: TemplateAddress,
        open_bounties: Vec<ComponentAddress>,
    }

    impl BountyBoard {
        /// Deploy the registry.
        ///
        /// * `bounty_template` — template address of the `Bounty` template.
        pub fn new(bounty_template: TemplateAddress) -> Component<Self> {
            Component::new(Self {
                bounty_template,
                open_bounties: Vec::new(),
            })
            .with_access_rules(ComponentAccessRules::new().default(rule!(allow_all)))
            .create()
        }

        /// Post a new bounty and register it on the board.
        ///
        /// Creates a `Bounty` component via `TemplateManager` and records its address.
        ///
        /// * `owner_badge` — the caller's badge NFT address; controls owner-only ops.
        /// * `owner_account` — account to receive remaining budget if cancelled.
        /// * `steps` — task steps to execute atomically on each run.
        /// * `fee` — initial fee budget.
        /// * `fee_per_run` — fee paid to the executor per run.
        /// * `min_epoch` — earliest epoch at which the bounty may execute (0 = immediately).
        /// * `interval_epochs` — `None` = one-shot; `Some(N)` = repeat every N epochs.
        ///
        /// Returns the address of the newly created `Bounty` component.
        pub fn post(
            &mut self,
            owner_badge: NonFungibleAddress,
            owner_account: ComponentAddress,
            steps: Vec<TaskStep>,
            fee: Bucket,
            fee_per_run: Amount,
            min_epoch: u64,
            interval_epochs: Option<u64>,
        ) -> ComponentAddress {
            let bounty_addr: ComponentAddress = TemplateManager::get(self.bounty_template).call(
                "new",
                args![owner_badge, owner_account, steps, fee, fee_per_run, min_epoch, interval_epochs],
            );
            self.open_bounties.push(bounty_addr);

            emit_event("BountyPosted", metadata![
                "bounty" => bounty_addr.to_string(),
                "min_epoch" => min_epoch.to_string(),
                "recurring" => interval_epochs.map(|n| n.to_string()).unwrap_or_else(|| "no".to_string()),
            ]);

            bounty_addr
        }

        /// Returns addresses of all registered bounties (active and recently inactive).
        /// Executor bots should call `bounty.is_eligible()` on each to filter candidates.
        pub fn list_open(&self) -> Vec<ComponentAddress> {
            self.open_bounties.clone()
        }

        /// Remove inactive bounties from the registry.
        ///
        /// Calls `is_active()` on each bounty and drops those that are cancelled or exhausted.
        /// Anyone can call this.
        pub fn prune(&mut self) {
            self.open_bounties.retain(|&addr| {
                let active: bool = ComponentManager::get(addr).call("is_active", args![]);
                active
            });
        }

        /// Total number of bounties ever registered on this board.
        pub fn total_registered(&self) -> u64 {
            self.open_bounties.len() as u64
        }
    }
}
