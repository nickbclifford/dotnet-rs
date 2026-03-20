using System;
using System.Threading;

public class Program {
    static int s_counter = 0;
    static object s_lock = new object();
    static volatile int s_thread_0_held = 0;
    static volatile int s_thread_1_tried_and_failed = 0;
    static volatile int s_thread_2_tried_and_succeeded = 0;

    static void Yield() { Thread.Sleep(1); }

    public static int Main() {
        // Simple manual thread ID assignment via Interlocked
        int id = Interlocked.Increment(ref s_counter) - 1;

        if (id == 0) {
            // Thread 0: Hold the lock until Thread 1 fails
            Monitor.Enter(s_lock);
            s_thread_0_held = 1;

            // Busy wait until Thread 1 confirms it failed its TryEnter
            while (s_thread_1_tried_and_failed == 0) {
                Yield();
            }

            Monitor.Exit(s_lock);
            s_thread_0_held = 2; // Signal that lock is released
            return 42;
        } else if (id == 1) {
            // Thread 1: Wait for Thread 0 to hold the lock, then try to enter with short timeout
            while (s_thread_0_held == 0) {
                Yield();
            }

            // TryEnter with 50ms timeout - should fail because Thread 0 is holding it
            bool entered = Monitor.TryEnter(s_lock, 50);
            if (entered) {
                // Error: should NOT have entered
                Monitor.Exit(s_lock);
                return 1;
            }

            // Signal to Thread 0 that we failed
            s_thread_1_tried_and_failed = 1;
            return 42;
        } else if (id == 2) {
            // Thread 2: Wait for Thread 0 to release the lock, then TryEnter with long timeout
            while (s_thread_0_held != 2) {
                Yield();
            }

            // TryEnter with 5000ms timeout - should succeed now that Thread 0 is done
            bool entered = Monitor.TryEnter(s_lock, 5000);
            if (!entered) {
                // Error: should have entered eventually
                return 2;
            }

            // Test re-entrancy with timeout
            bool reentered = Monitor.TryEnter(s_lock, 100);
            if (!reentered) {
                Monitor.Exit(s_lock);
                return 3;
            }
            Monitor.Exit(s_lock);

            Monitor.Exit(s_lock);
            s_thread_2_tried_and_succeeded = 1;
            return 42;
        }

        return 42;
    }
}
