using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

class Program
{
    static object? Root;
    static WeakReference? Live;
    static WeakReference? Dead;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static bool Create()
    {
        Root = new object();
        object target = new object();
        Live = new WeakReference(Root);
        Dead = new WeakReference(target);
        bool valid = Live.IsAlive && Dead.IsAlive
            && Object.ReferenceEquals(Live.Target, Root)
            && Object.ReferenceEquals(Dead.Target, target);
        GC.KeepAlive(target);
        return valid;
    }

    static int Main()
    {
        bool ok = Create();
        var normal = GCHandle.Alloc(Root!, GCHandleType.Normal);
        var pin = GCHandle.Alloc(new byte[8], GCHandleType.Pinned);
        ok = ok && Object.ReferenceEquals(normal.Target, Root) && pin.Target != null
            && pin.AddrOfPinnedObject() != IntPtr.Zero;
        object replacement = new object();
        normal.Target = replacement;
        ok = ok && Object.ReferenceEquals(normal.Target, replacement);
        normal.Target = Root;
        for (int i = 0; i < 3; i++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
            ok = ok && Live!.IsAlive && Object.ReferenceEquals(Live.Target, Root);
        }
        ok = ok && !Dead!.IsAlive && Dead.Target == null;
        normal.Free();
        pin.Free();
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
