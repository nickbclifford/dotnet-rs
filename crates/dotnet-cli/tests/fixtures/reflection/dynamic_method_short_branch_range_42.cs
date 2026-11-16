using System;
using System.Reflection.Emit;

class Program
{
    static void Probe(string name, bool backward, int padding, bool shortBranch)
    {
        var method = new DynamicMethod(name, typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var target = il.DefineLabel();
        if (backward) il.MarkLabel(target);
        if (!backward) il.Emit(shortBranch ? OpCodes.Br_S : OpCodes.Br, target);
        for (int i = 0; i < padding; i++) il.Emit(OpCodes.Nop);
        if (backward)
        {
            il.Emit(OpCodes.Ldc_I4_1);
            il.Emit(shortBranch ? OpCodes.Brfalse_S : OpCodes.Brfalse, target);
        }
        else il.MarkLabel(target);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ret);
        try { Console.WriteLine(name + ":" + ((Func<int>)method.CreateDelegate(typeof(Func<int>)))()); }
        catch (Exception e) { Console.WriteLine(name + ":" + e.GetType().Name); }
    }

    static int Main()
    {
        Probe("forward127", false, 127, true);
        Probe("forward128", false, 128, true);
        Probe("backward128", true, 125, true);
        Probe("backward129", true, 126, true);
        Probe("forwardLong", false, 128, false);
        Probe("backwardLong", true, 126, false);
        return 42;
    }
}
