using System;
using System.Reflection.Emit;

delegate ref int RefIdentity(ref int value);
class Program
{
    static int Main()
    {
        var byref = typeof(int).MakeByRefType();
        var method = new DynamicMethod("Identity", byref, new[] { byref });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ret);
        var identity = (RefIdentity)method.CreateDelegate(typeof(RefIdentity));
        int value = 0;
        identity(ref value) = 42;
        Console.WriteLine(value);
        return value;
    }
}
