using System;
using System.Reflection.Emit;

class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Generated", typeof(void), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Newobj, typeof(InvalidOperationException).GetConstructor(Type.EmptyTypes));
        il.Emit(OpCodes.Throw);
        var action = (Action)method.CreateDelegate(typeof(Action));
        Console.WriteLine("method:" + action.Method.Name + ":" + (action.Method.DeclaringType == null));
        try { action(); }
        catch (Exception error)
        {
            string trace = error.StackTrace;
            Console.WriteLine("frame:" + trace.Contains("at Generated()"));
            Console.WriteLine("holder:" + trace.Contains("DynamicMethods.Generated"));
        }
        return 42;
    }
}
