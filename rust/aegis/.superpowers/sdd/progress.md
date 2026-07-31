
- Task 6: complete (6a f133eb54 + 6b e2fc1652 + gate-fix 6032a5c5, both reviews Approved)
  - Full unification: Telegram + Matrix events → BotEvent → dispatch_event. callback.rs + telegram message.rs deleted. state_ops lang redirect + BotAdapter::set_system_locale shim added.
  - Known plan gaps (flag for final review): (1) security-file UPLOAD/hash/persist flow not migrated (unified commands.rs SetSecurityFile only sends prompt); (2) rich Matrix text subcommands (xray/singbox/ops/destruct/schedule/warp) fall through to BotEvent::Message unhandled.
  - Follow-ups: remove empty `adapters/telegram/handlers/` module + its `mod handlers` decl in main.rs (Task 9); orphaned `matrix::commands::Command`/`parse` (Task 9); `pub use dispatch::dispatch_event` re-export in shared/mod.rs (minimal, necessary); dead_code allows in bootstrap.rs/utils (Task 9).
- Task 7: complete (8ee98712, review Approved-by-controller) — legacy destruct_flow.rs deleted, app/mod.rs decl removed, no dangling refs. 536 tests (lost 6 pure-logic tests not ported in Task 3; shared has 7 behavioral tests).
Final review fix: complete (commit a03757d, re-review clean)
