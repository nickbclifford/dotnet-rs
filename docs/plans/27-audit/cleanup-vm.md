# VM test cleanup

Dispositions for the VM (VM-*), inline-test (VI-*) and pointer-provenance (VP-*) findings of the
[27a audit](../27a-test-audit.md) (the finding texts are in git).

## Dispositions

| Finding | Change and retained contract |
|---|---|
| VM-2 | Fuzz execution now propagates entrypoint setup errors and retains the real `ExecutorResult`. Test-only tracing records instruction IPs only when the current frame's `MethodDescription` equals the generated `FuzzMain` entrypoint. A constructor control independently records dispatched declaring types/methods/IPs, proves helper IP 3 ran, and proves the same skipped FuzzMain IP is absent from the filtered trace. Its mock corlib provides the minimal `System.ValueType`/`System.Enum` ancestry needed by the real `newobj` path. Named cases assert reached entrypoint instructions and expected return, VM error, or managed throw; division by zero is documented as VM error-path safety coverage. |
| VM-3 | Named semantic corpus entries use a versioned `DVMSEED1` ID format decoded by `FuzzProgram::arbitrary`; independent tests compare arithmetic, memory, and branch instructions and results against explicit expectations. The deep-stack seed reaches depth 50 and drains before return; stack-dup and unchecked-add seeds also return with a clean stack. Legacy opaque bytes are relabeled and preserved byte-for-byte. The corpus generators refuse to overwrite differing existing inputs. |
| VM-4 | Generated registrations are exercised against loaded CoreLib metadata. Tests resolve a RuntimeHelpers handler, dispatch loaded `Math.Sqrt(double)` and check `3.0`, and check `System.Array.GetValue(int)`, `GetValue(long)`, and `GetValue(int[])` dispatch/filter matches and mismatches. No generated hashes or handler source spellings are the runtime oracle. |
| VM-5 | Tail-call depth is reported only after observing `StepResult::Return`; host errors, managed throws, and step-budget exhaustion fail the test. The tail and non-tail depth witnesses remain. |
| VM-6 | The static object-reference test retains null as a control and stores/loads a non-null array reference through a real static field, checking object identity in the returned result. |
| VM-7 | Removed the source-text scan. The production Mark command test remains with a live positive control and stale, replacement-generation, and foreign-generation rejection cases. Root added its exact selector to the multithreaded Miri singleton manifest. |
| VM-8 | Ring-buffer events use alternating opcodes and distinct IPs; the test checks recent IPs 5–14 in order and a partially filled buffer's exact output. |
| VM-9 | The contended monitor test verifies the holder acquired the lock, waits for a production contention hook, then releases through channels and observes waiter acquisition. It no longer relies on an ignored acquisition result or a scheduling sleep. |
| VI-1 | Local records use two real arena parents and distinct thread/delegate/argument roots. The test retains an unstarted record with a `None` start argument, checks parent-local contents, and observes completed-record pruning in each parent. |
| VI-2 | Canonical queue tests retain completion handles and verify successful completion survives waiter drop and shutdown remains terminal after finishing already-taken work. |
| VI-3 | Consolidated constructor checks around distinct non-Missing and Missing handlers with expected kind/reason fields; removed duplicated enum inequality and storage-only assertions. |
| VI-4 | The blocking registry check selects semantic `(type, member)` names, requires the known blocking signatures, and asserts discovered matching entries have no sync fallback. It has no hash suffix selection or fixed overload quota. |
| VI-5 | STW tests now wait for a hook after the production failed try-lock before releasing coordination, then assert the resulting request behavior with bounded channel cleanup. |
| VP-1 | p35 initializes adjacent value-type fields to distinct nonzero values using the local's address, then reads the target through `ldflda`/`ldind`. |
| VP-2 | p29 keeps the evaluation stack empty across `localloc`, checks zero-size alignment without dereferencing the zero extent, and reads/writes first and last bytes only in nonzero allocations. |
| VP-3 | p09 and p41 make nested callees push and pop 1,024 values. Their observers require actual evaluation-stack capacity growth while retaining p09's pointer roundtrip and p41's constructor receiver/collection assertions. |
| VP-4 | p19 adds `IndexOutOfRangeException` to its mock corlib, observes `MethodThrew`, and distinguishes the expected managed exception from host errors and panics. Checked array-stride multiplication maps an unrepresentable offset to the managed bounds exception. After passing both native and Miri configurations, p19 was enabled as pointer control shard 45. |

## Checks

The changed pointer control shards are 32 (p35), 26 (p29), 36 (p09), 41 (p41) and 45 (p19), under
both `no-features` and `multithreading`; runner commands are in [CI.md](../../CI.md#pointer-lane).
The fuzz-harness tests (`fuzzing::tests::`) run natively in the `fuzzing` configuration and passed
under Miri as guarded two-case groups; executor corpus replay passed with AddressSanitizer
(LeakSanitizer disabled locally because the sandbox's ptrace restriction prevents it).
