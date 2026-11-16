using System;
using System.Reflection.Emit;
using System.Runtime.InteropServices;

class Program
{
    [DllImport("libc")]
    private static extern IntPtr dlsym(IntPtr handle, byte[] symbol);

    static int Main()
    {
        IntPtr target = dlsym(IntPtr.Zero, new byte[] { 97, 98, 115, 0 });
        var method = new DynamicMethod("NativeIndirect", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, -42);
        il.Emit(OpCodes.Ldc_I8, target.ToInt64());
        il.Emit(OpCodes.Conv_I);
        il.EmitCalli(OpCodes.Calli, CallingConvention.Cdecl, typeof(int), new[] { typeof(int) });
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}
