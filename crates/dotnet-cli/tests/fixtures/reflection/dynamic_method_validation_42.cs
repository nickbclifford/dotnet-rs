using System;
using System.Reflection.Emit;
class Program
{
    static void Probe(string name, Action<ILGenerator> action)
    {
        var method = new DynamicMethod(name, typeof(void), Type.EmptyTypes);
        var il = method.GetILGenerator();
        try { action(il); _ = method.CreateDelegate(typeof(Action)); Console.WriteLine(name + ":none"); }
        catch (Exception e) { Console.WriteLine(name + ":" + e.GetType().Name); }
    }
    static int Main()
    {
        Probe("unmarked", il => { var label = il.DefineLabel(); il.Emit(OpCodes.Br, label); il.Emit(OpCodes.Ret); });
        Probe("twice", il => { var label = il.DefineLabel(); il.MarkLabel(label); il.MarkLabel(label); });
        Probe("unclosed", il => { il.BeginExceptionBlock(); il.Emit(OpCodes.Ret); });
        Probe("end", il => il.EndExceptionBlock());
        Probe("catch", il => il.BeginCatchBlock(typeof(Exception)));
        Probe("finally", il => il.BeginFinallyBlock());
        Probe("filter", il => il.BeginExceptFilterBlock());
        Probe("fault", il => il.BeginFaultBlock());
        Probe("empty", il => { il.BeginExceptionBlock(); il.EndExceptionBlock(); });
        Probe("nullcatch", il => { il.BeginExceptionBlock(); il.BeginCatchBlock(null); });
        return 42;
    }
}
