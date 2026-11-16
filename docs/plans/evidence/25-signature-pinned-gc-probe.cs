using System;
using System.Reflection;
using System.Reflection.Emit;
class Program
{
    public static unsafe int Observe(byte[] bytes)
    {
        nint before;
        fixed (byte* p = bytes) before = (nint)p;
        GC.Collect(2, GCCollectionMode.Forced, blocking: true, compacting: true);
        fixed (byte* p = bytes) return before == (nint)p ? 0 : 1;
    }
    static Func<byte[],int> Build(bool pinned)
    {
        var dm = new DynamicMethod("Probe", typeof(int), new[] { typeof(byte[]) });
        var il = dm.GetILGenerator();
        var sig = SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int));
        sig.AddArgument(typeof(byte[]), pinned);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(Observe))!);
        il.Emit(OpCodes.Calli, sig);
        il.Emit(OpCodes.Ret);
        return (Func<byte[],int>)dm.CreateDelegate(typeof(Func<byte[],int>));
    }
    static int Main()
    {
        var plain = Build(false); var pinned = Build(true);
        int ordinaryMoved = 0, pinnedMoved = 0;
        for (int pass = 0; pass < 20; pass++)
        {
            var holes = new byte[8000][];
            for (int i = 0; i < holes.Length; i++) holes[i] = new byte[256];
            byte[] subject = holes[^2];
            for (int i = 0; i < holes.Length; i += 2) holes[i] = null!;
            int moved = (pass % 2 == 0 ? plain : pinned)(subject);
            if (pass % 2 == 0) ordinaryMoved += moved; else pinnedMoved += moved;
            GC.KeepAlive(holes);
        }
        Console.WriteLine($"ordinary moved {ordinaryMoved}/10; pinned-marker moved {pinnedMoved}/10");
        return 0;
    }
}
