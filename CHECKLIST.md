# Refactor Checklist — dotnet-rs

Machine-readable checklist for the refactor plan in REVIEW.md.
Each step is small enough for one agent session. References are stable anchors in REVIEW.md.

## Phase 1: Documentation Fix-up
- [x] 1.1 Remove the 8 stale `docs/p3_s1_trait_inventory.md` references from source files (or create the document) — refs REVIEW.md#F-DEAD-004
- [x] 1.2 Document `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES` / `DOTNET_VM_EXTRA_INTRINSIC_SOURCES` extension points in `docs/BUILD_TIME_CODE_GENERATION.md` — refs REVIEW.md#F-BUILD-002
- [x] 1.3 Add `dotnet-benchmarks`, `dotnet-build-tools`, `dotnet-simd` to `docs/ARCHITECTURE.md` dependency hierarchy — refs REVIEW.md#F-DOC-002
- [x] 1.4 Expand the incomplete TODO items in `docs/EXCEPTION_HANDLING.md` with brief inline explanations — refs REVIEW.md#F-DOC-003
- [x] 1.5 Rename the "experimental feature smoke tests" section in `check.sh` to "prototype compilation guards" and add a scope comment — refs REVIEW.md#F-BUILD-001
- [x] 1.6 Add missing `../dotnet-intrinsics-simd/src` default intrinsic root to `docs/BUILD_TIME_CODE_GENERATION.md` root list — discovered during step 1.2 execution

## Phase 2: Remove Vestigial Trait Plumbing
- [ ] 2.1 Remove empty `CallOps<'gc>` trait from `dotnet-vm-ops/src/ops.rs`; remove `: CallOps<'gc>` from `VmCallOps`; remove blank impl in `context.rs`; update all re-exports — refs REVIEW.md#F-DEAD-001
- [ ] 2.2 Remove `AtomicMemoryHost<'gc>` from `dotnet-intrinsics-threading/src/lib.rs`; replace `+ AtomicMemoryHost<'gc>` with `+ RawMemoryOps<'gc>` in `ThreadingIntrinsicHost`; rename `threading_*` calls to `RawMemoryOps` calls in handler files — refs REVIEW.md#F-DEAD-002

## Phase 3: Error Propagation in Hot Paths
- [ ] 3.1 Replace `self.frame_stack.pop().unwrap()` in `context.rs:229` (`return_frame`) with a match that returns `StepResult::Error(...)` on empty stack — refs REVIEW.md#F-IDIOM-001
- [ ] 3.2 Replace `expect("Thread arena not initialized")` panics in `executor.rs:62,77` with graceful error returns — refs REVIEW.md#F-IDIOM-002

## Phase 4: context.rs Trait-Impl Split
- [ ] 4.1 Create `crates/dotnet-vm/src/stack/context_ops.rs`; move all `impl VesContext` blocks for external traits out of `context.rs` into it; update `stack/mod.rs` — refs REVIEW.md#F-OVER-001
- [ ] 4.2 Run `check.sh` under all feature combinations to verify no regressions from the split — refs REVIEW.md#F-OVER-001

## Phase 5: TypeComparer — ResolutionS by Reference
- [ ] 5.1 Change all `TypeComparer` method signatures in `comparer.rs` from `res1: ResolutionS` to `res1: &ResolutionS`; remove the 38 `.clone()` calls on `res1`/`res2` within the file — refs REVIEW.md#F-TYPES-001
- [ ] 5.2 Update callers of `TypeComparer` methods in `dotnet-runtime-resolver/` to pass `&ResolutionS` references — refs REVIEW.md#F-TYPES-001

## Phase 6: Address Open TODOs
- [ ] 6.1 Implement argv initialization: wire `std::env::args()` to managed `Main(string[] args)` entry point in `executor.rs:165` — refs REVIEW.md#F-TYPES-002
- [ ] 6.2 Encode element type and rank in `RuntimeType` for arrays/vectors (replace TODO at `intrinsics/mod.rs:514`) — refs REVIEW.md#F-TYPES-002
- [ ] 6.3 Document span non-ordinal comparison limitation at `span/equality.rs:320` with a concrete explanation of what implementation would require — refs REVIEW.md#F-TYPES-002
