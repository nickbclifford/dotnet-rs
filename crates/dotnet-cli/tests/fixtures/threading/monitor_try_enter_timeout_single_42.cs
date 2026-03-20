using System;
using System.Threading;

// Single-threaded Monitor.TryEnter coverage fixture.
//
// Tests TryEnter semantics that are meaningful without cross-thread contention:
// uncontested acquisition with timeout, reëntrant acquisition with timeout, and
// release-then-reacquire.  The multi-threaded contention scenario (TryEnter that
// must actually wait or time out against another holder) is covered by
// monitor_try_enter_timeout_42, which requires the `multithreading` feature and
// is correctly ignored under --no-default-features.
public class Program {
    static object s_lock = new object();

    public static int Main() {
        // Uncontested TryEnter with timeout should succeed immediately.
        bool entered = Monitor.TryEnter(s_lock, 100);
        if (!entered) return 1;

        // Re-entrant TryEnter while already holding the lock must succeed.
        bool reentered = Monitor.TryEnter(s_lock, 100);
        if (!reentered) {
            Monitor.Exit(s_lock);
            return 2;
        }

        // Release inner hold.
        Monitor.Exit(s_lock);

        // Release outer hold.
        Monitor.Exit(s_lock);

        // Re-acquire after full release should succeed.
        bool reacquired = Monitor.TryEnter(s_lock, 100);
        if (!reacquired) return 3;

        // TryEnter with zero timeout on an uncontested lock should also succeed.
        bool zeroTimeout = Monitor.TryEnter(s_lock, 0);
        if (!zeroTimeout) {
            Monitor.Exit(s_lock);
            return 4;
        }

        Monitor.Exit(s_lock); // inner zero-timeout acquire
        Monitor.Exit(s_lock); // reacquired

        return 42;
    }
}
