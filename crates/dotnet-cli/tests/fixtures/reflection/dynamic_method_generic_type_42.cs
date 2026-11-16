using System;
using System.Collections.Generic;
using System.Reflection.Emit;

class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Add", typeof(int), new[] { typeof(List<int>) });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Callvirt, typeof(List<int>).GetMethod("Add")!);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldc_I4_0);
        il.Emit(OpCodes.Callvirt, typeof(List<int>).GetProperty("Item")!.GetMethod!);
        il.Emit(OpCodes.Ret);
        var result = ((Func<List<int>, int>)method.CreateDelegate(typeof(Func<List<int>, int>)))(new List<int>());
        Console.WriteLine(result);
        return result;
    }
}
