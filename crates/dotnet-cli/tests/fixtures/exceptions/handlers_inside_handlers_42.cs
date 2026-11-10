using System;

public class Program {
    static readonly object Gate = new object();
    static int Normal() {
        int r = 0;
        try { r += 1; } finally { try { r += 2; } finally { r += 4; } }
        return r;
    }
    static int Pending() {
        int r = 0;
        try {
            try { throw new InvalidOperationException(); }
            finally { try { r += 1; } finally { GC.Collect(); r += 2; } }
        } catch (InvalidOperationException) { r += 4; }
        return r;
    }
    static int Locked() {
        int r = 0;
        try {
            try { throw new InvalidOperationException(); }
            finally { lock (Gate) { r += 1; } }
        } catch (InvalidOperationException) { r += 2; }
        return r;
    }
    static int Caught() {
        int r = 0;
        try {
            try { throw new InvalidOperationException(); }
            finally {
                try { throw new ArgumentException(); } catch (ArgumentException) { r += 1; }
                r += 2;
            }
        } catch (InvalidOperationException) { r += 4; }
        return r;
    }
    static int Callee() { try { return 1; } finally { GC.KeepAlive(null); } }
    static int CallInFinally() { int r = 0; try {} finally { r = Callee(); } return r; }
    static int FinallyInCatch() { int r = 0; try { throw new Exception(); } catch { try { r++; } finally { r++; } } return r; }
    static int Escape() {
        int r = 0;
        try {
            try { throw new InvalidOperationException(); }
            finally {
                try { try { throw new ArgumentException(); } finally { r++; } }
                finally { r++; }
            }
        } catch (ArgumentException) { r += 4; } catch (InvalidOperationException) { return -1; }
        // An unrelated leave must not resurrect the replaced exception.
        try { r++; } finally { r++; }
        return r;
    }
    static int Implicit() {
        int r = 0, zero = 0;
        try { try { throw new InvalidOperationException(); }
            finally { try { r += 1 / zero; } catch (DivideByZeroException) { r += 1; } }
        } catch (InvalidOperationException) { r += 2; }
        return r;
    }
    static int Deep() {
        int r = 0;
        try {} finally {
            try {} finally {
                try {} finally { r++; }
                r++;
            }
            r++;
        }
        return r;
    }
    public static int Main() {
        if (Normal() != 7 || Pending() != 7 || Locked() != 3 || Caught() != 7 ||
            CallInFinally() != 1 || FinallyInCatch() != 2 || Escape() != 8 || Implicit() != 3 || Deep() != 3) return 1;
        return 42;
    }
}
