using System;
using System.Reflection;
using System.Runtime.InteropServices;

// Every way of reaching a non-generic callee from a generic method must resolve the callee with
// its own (empty) method generics. Span<byte> reads bottom out in the byte-sized Memmove
// intrinsic, which previously sized its copy from the leaked T = long.
public unsafe class Program {
    delegate void CopyDelegate(UnmanagedMemoryStream s, byte[] d);

    interface ICopier { void Copy(UnmanagedMemoryStream s, byte[] d); }
    abstract class BaseCopier { public abstract void Copy(UnmanagedMemoryStream s, byte[] d); }
    class Copier : BaseCopier, ICopier {
        public override void Copy(UnmanagedMemoryStream s, byte[] d) { s.Read(new Span<byte>(d, 0, 4)); }
        void ICopier.Copy(UnmanagedMemoryStream s, byte[] d) { s.Read(new Span<byte>(d, 0, 4)); }
    }
    class Ctor {
        public Ctor(UnmanagedMemoryStream s, byte[] d) { s.Read(new Span<byte>(d, 0, 4)); }
    }
    class Cctor {
        static Cctor() { Program.Src.Read(new Span<byte>(Program.Dst, 0, 4)); }
        public static void Touch() { }
    }

    static UnmanagedMemoryStream Src;
    static byte[] Dst;

    static void StaticCopy(UnmanagedMemoryStream s, byte[] d) { s.Read(new Span<byte>(d, 0, 4)); }

    static int Verify(byte[] d) {
        for (int i = 0; i < 4; i++) if (d[i] != i + 1) return 1;
        for (int i = 4; i < d.Length; i++) if (d[i] != 0xEE) return 2;
        return 0;
    }

    static byte[] Fresh() { var d = new byte[64]; for (int i = 0; i < d.Length; i++) d[i] = 0xEE; return d; }

    static int Run<T>(byte* p, int which) {
        var d = Fresh();
        var s = new UnmanagedMemoryStream(p, 64);
        switch (which) {
            case 0: StaticCopy(s, d); break;
            case 1: ((CopyDelegate)StaticCopy)(s, d); break;
            case 2: new Copier().Copy(s, d); break;
            case 3: ((BaseCopier)new Copier()).Copy(s, d); break;
            case 4: ((ICopier)new Copier()).Copy(s, d); break;
            case 5: new Ctor(s, d); break;
            case 6: typeof(Program).GetMethod("StaticCopy", BindingFlags.NonPublic | BindingFlags.Static).Invoke(null, new object[] { s, d }); break;
            case 7: { if (typeof(T) != typeof(long)) { StaticCopy(s, d); break; } Src = s; Dst = d; Cctor.Touch(); break; }
            case 8: { CopyDelegate c = new Copier().Copy; c(s, d); break; }
            case 9: { Func<UnmanagedMemoryStream, byte[], Action> f = (a, b) => () => a.Read(new Span<byte>(b, 0, 4)); f(s, d)(); break; }
        }
        return Verify(d);
    }

    public static int Main() {
        byte* s = (byte*)NativeMemory.Alloc(64);
        for (int i = 0; i < 64; i++) s[i] = (byte)(i + 1);
        for (int which = 0; which <= 9; which++) {
            if (Run<long>(s, which) != 0) return 10 + which;
            if (Run<Guid>(s, which) != 0) return 30 + which;
        }
        return 42;
    }
}
