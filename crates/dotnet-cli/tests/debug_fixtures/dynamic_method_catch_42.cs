using System;
using System.Reflection.Emit;

// Plan 24 slice 4: exercise the dynamic resolver's exception regions and catch type token.
public static class Program
{
    public static int Main()
    {
        var method = new DynamicMethod("Catch", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var result = il.DeclareLocal(typeof(int));
        il.BeginExceptionBlock();
        il.Emit(OpCodes.Newobj, typeof(InvalidOperationException).GetConstructor(Type.EmptyTypes)!);
        il.Emit(OpCodes.Throw);
        il.BeginCatchBlock(typeof(InvalidOperationException));
        il.Emit(OpCodes.Pop);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Stloc, result);
        il.EndExceptionBlock();
        il.Emit(OpCodes.Ldloc, result);
        il.Emit(OpCodes.Ret);

        var run = (Func<int>)method.CreateDelegate(typeof(Func<int>));
        int value = run();
        Console.WriteLine($"Catch: {value}");
        return value == 42 ? 42 : 1;
    }
}
