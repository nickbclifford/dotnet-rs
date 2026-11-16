using System;
using System.Reflection.Emit;

// Emit ldarg.0; ldarg.1; add; ret and run it through both Invoke and CreateDelegate.
public static class Program
{
    public static int Main()
    {
        var method = new DynamicMethod("Add", typeof(int), new[] { typeof(int), typeof(int) });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldarg_1);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Ret);

        var invoked = (int)method.Invoke(null, new object[] { 19, 23 })!;
        var add = (Func<int, int, int>)method.CreateDelegate(typeof(Func<int, int, int>));
        var delegated = add(20, 22);
        Console.WriteLine($"Invoke: {invoked}; delegate: {delegated}");
        return invoked == 42 && delegated == 42 ? 42 : 1;
    }
}
