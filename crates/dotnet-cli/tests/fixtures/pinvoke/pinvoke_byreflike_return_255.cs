// Independent recreation (not recovered originals): a P/Invoke returning a
// byref-like aggregate by value. VM storage gives the ref field three words,
// while the native ABI writes one pointer word, so the native `rem` lands in
// managed-pointer word 1 (the arena identity word) and `Rem` reads garbage.
// Expected disposition: the VM rejects the call before invoking native code
// (exit 255, see `pinvoke_byreflike_return_rejected` in fixtures.rs).
using System.Runtime.InteropServices;

public ref struct LdivRef {
    public ref long Quot;
    public long Rem;
}

public class Program {
    [DllImport("libc", EntryPoint = "ldiv")]
    static extern LdivRef ldiv(long numer, long denom);

    public static int Main() {
        LdivRef r = ldiv(100, 7);
        // Only `Rem` is read: decoding `Quot` would decode native bytes as a
        // managed pointer. A correct by-value layout would give Rem == 2.
        return r.Rem == 2 ? 42 : 7;
    }
}
