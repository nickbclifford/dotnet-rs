using System;
using System.Reflection.Emit;

public class Box { public int Value; public static int Shared; }
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Fields", typeof(int), new[] { typeof(Box) });
        var il = method.GetILGenerator();
        var value = typeof(Box).GetField("Value")!;
        var shared = typeof(Box).GetField("Shared")!;
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldc_I4, 40);
        il.Emit(OpCodes.Stfld, value);
        il.Emit(OpCodes.Ldc_I4_2);
        il.Emit(OpCodes.Stsfld, shared);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldfld, value);
        il.Emit(OpCodes.Ldsfld, shared);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Ret);
        var result = ((Func<Box, int>)method.CreateDelegate(typeof(Func<Box, int>)))(new Box());
        Console.WriteLine(result);
        return result;
    }
}
