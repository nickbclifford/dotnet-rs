using System;
using System.Reflection.Emit;

class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Sum", typeof(int), new[] { typeof(int) });
        var il = method.GetILGenerator();
        var sum = il.DeclareLocal(typeof(int));
        var loop = il.DefineLabel();
        var done = il.DefineLabel();
        il.MarkLabel(loop);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Brfalse_S, done);
        il.Emit(OpCodes.Ldloc, sum);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Stloc, sum);
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldc_I4_1);
        il.Emit(OpCodes.Sub);
        il.Emit(OpCodes.Starg_S, (byte)0);
        il.Emit(OpCodes.Br, loop);
        il.MarkLabel(done);
        il.Emit(OpCodes.Ldloc, sum);
        il.Emit(OpCodes.Ldc_I4, 21);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Ret);
        int result = ((Func<int, int>)method.CreateDelegate(typeof(Func<int, int>)))(6);
        Console.WriteLine(result);
        return result;
    }
}
