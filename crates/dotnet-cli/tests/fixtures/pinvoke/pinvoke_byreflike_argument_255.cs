// Independent recreation (not recovered originals): a P/Invoke taking a
// byref-like aggregate by value. The raw three-word managed-pointer storage is
// handed to libffi, which declares the ref field as one native pointer, so the
// native callee receives encoded word 0 and word 1 (for a heap ref, the byte
// offset; for a cross-arena ref, the arena identity word) instead of
// (address, Denom).
// Expected disposition: the VM rejects the call before invoking native code
// (exit 255, see `pinvoke_byreflike_argument_rejected` in fixtures.rs).
using System.Runtime.InteropServices;

public ref struct DivArgs {
    public ref long Numer;
    public long Denom;
}

[StructLayout(LayoutKind.Sequential)]
public struct LdivT {
    public long Quot;
    public long Rem;
}

public unsafe class Program {
    // On SysV x86-64 a 16-byte INTEGER-class aggregate is passed in the same
    // two registers as `ldiv(long, long)`.
    [DllImport("libc", EntryPoint = "ldiv")]
    static extern LdivT ldiv(DivArgs args);

    public static int Main() {
        // Element 1 keeps the heap ref's word-1 byte offset nonzero, so the
        // pre-fix native division cannot trap on a zero divisor.
        long[] cells = new long[2];
        fixed (long* cell = &cells[1]) {
            DivArgs args = default;
            args.Numer = ref cells[1];
            args.Denom = 1;
            LdivT r = ldiv(args);
            // A correct ABI would divide the element's address by 1.
            return r.Quot == (long)cell && r.Rem == 0 ? 42 : 7;
        }
    }
}
