# tari-kairos

> **Καιρός** — the Greek god of the opportune moment; the fleeting instant when conditions are exactly right for action.

Epoch-gated deferred execution templates for the [Tari Ootle](https://github.com/tari-project/tari-ootle) network.

Post one-shot or recurring on-chain tasks with a fee vault. Executor bots scan the board, wait for the right epoch, and claim the bounty by running the task atomically.

---

## Templates

### `BountyBoard`

A global registry that holds no funds of its own. Executor bots have a single well-known address to scan.

| Method | Access | Description |
|---|---|---|
| `new(bounty_template)` | — | Deploy the registry |
| `post(owner_badge, owner_account, steps, fee, fee_per_run, min_epoch, interval_epochs)` | anyone | Create a `Bounty` and register it |
| `list_open()` | anyone | All registered bounty addresses |
| `prune()` | anyone | Remove inactive/exhausted bounties |
| `total_registered()` | anyone | Running count of registered bounties |

### `Bounty`

A self-contained fee-vault component that stores cross-component task steps and pays a configurable fee to any executor that triggers it at the right epoch.

| Method | Access | Description |
|---|---|---|
| `execute(executor_account)` | anyone | Run all steps and pay the fee |
| `is_eligible()` | anyone | `true` if executable in the current epoch |
| `is_active()` | anyone | `true` if not cancelled or exhausted |
| `info()` | anyone | Read-only state summary |
| `top_up(funds)` | owner | Add budget; reactivates if exhausted |
| `set_fee_per_run(amount)` | owner | Change the per-run fee |
| `set_interval(epochs)` | owner | Switch between one-shot and recurring |
| `cancel()` | owner | Deactivate and refund remaining budget |

---

## Execution modes

**One-shot** (`interval_epochs = None`)
The task steps execute once when `current_epoch >= min_epoch`, then the bounty deactivates automatically.

**Recurring / cron** (`interval_epochs = Some(N)`)
The steps re-execute every N epochs for as long as the vault can cover `fee_per_run`. The owner can top up at any time to extend the run.

---

## Task steps

Each `TaskStep` describes a single cross-component call:

```rust
pub struct TaskStep {
    pub target: ComponentAddress,  // component to call
    pub method: String,            // method name
    pub static_args: Vec<Vec<u8>>, // pre-encoded (BOR) args known at registration time
    pub append_executor: bool,     // if true, executor's ComponentAddress is appended as last arg
}
```

Steps execute atomically — a panic in any step reverts the entire transaction.

---

## Workflow

```
1. Deploy BountyBoard once (or reuse an existing one).

2. Post a bounty:
   BountyBoard::post(owner_badge, owner_account, steps, fee_bucket, fee_per_run, min_epoch, interval)

3. Executor bot scans:
   let open = board.list_open();
   for addr in open {
       if bounty.is_eligible() { bounty.execute(my_account); }
   }

4. Owner maintenance:
   bounty.top_up(bucket)          // add budget
   bounty.set_fee_per_run(amount) // adjust fee
   bounty.set_interval(Some(n))   // change cadence
   bounty.cancel()                // deactivate + refund

5. Housekeeping:
   board.prune()  // remove exhausted/cancelled bounties
```

---

## License

[BSD-3-Clause](LICENSE)
