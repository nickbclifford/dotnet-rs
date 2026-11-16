using System;
using System.Linq.Expressions;
using System.Reflection;
unsafe class Program
{
    private delegate void RefDelegate(ref int value, int a, int b);
    private delegate int PointerDelegate(int* p, int a, int b);
    private delegate int* PointerReturn(int* p, int a, int b);
    static int Main()
    {
        // The VM routes this framework helper through its support table. CoreCLR calls the
        // framework helper directly; Expression.Parameter itself rejects pointer types.
        Type helper = Type.GetType("DotnetRs.ObjectArrayDelegates");
        bool support = helper != null;
        if (!support) helper = typeof(Expression).Assembly.GetType("System.Dynamic.Utils.DelegateHelpers");
        var create = helper.GetMethod(support ? "Create" : "CreateObjectArrayDelegate", BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public);
        Func<object[], object> handler = args => { args[0] = 42; throw new InvalidOperationException(); };
        var del = (RefDelegate)create.Invoke(null, new object[] { typeof(RefDelegate), handler });
        int value = 1;
        try { del(ref value, 0, 0); } catch (InvalidOperationException) { }
        if (value != 42) return 1;
        handler = args => (int)(nint)args[0] + (int)args[1] + (int)args[2];
        var pointer = (PointerDelegate)create.Invoke(null, new object[] { typeof(PointerDelegate), handler });
        if (pointer((int*)10, 20, 12) != 42) return 2;
        handler = args => (nint)42;
        var pointerReturn = (PointerReturn)create.Invoke(null, new object[] { typeof(PointerReturn), handler });
        if ((nint)pointerReturn((int*)0, 0, 0) != 42) return 3;
        Console.WriteLine("ok"); return 42;
    }
}
