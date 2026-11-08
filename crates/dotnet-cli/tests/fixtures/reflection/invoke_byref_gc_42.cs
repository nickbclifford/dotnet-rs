using System;
using System.Reflection;

public static class Target
{
    public static void Update(ref int value, ref object item, out int created)
    {
        GC.Collect();
        GC.WaitForPendingFinalizers();
        value += 2;
        item = "changed";
        created = 40;
    }
}

public static class Program
{
    public static int Main()
    {
        var args = new object[] { 40, "before", null! };
        var result = typeof(Target).GetMethod(nameof(Target.Update))!.Invoke(null, args);
        if (result != null) return 1;
        if ((int)args[0] != 42 || (string)args[1] != "changed" || (int)args[2] != 40) return 2;
        return 42;
    }
}
