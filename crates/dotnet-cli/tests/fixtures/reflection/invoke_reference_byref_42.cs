using System;

public sealed class Holder { public int Value; }
public static class Target
{
    public static void Update(ref string text, ref Holder item)
    {
        text = "after";
        item = new Holder { Value = 42 };
        GC.Collect();
    }
}
public static class Program
{
    public static int Main()
    {
        object[] args = { "before", new Holder() };
        typeof(Target).GetMethod(nameof(Target.Update))!.Invoke(null, args);
        return (string)args[0] == "after" && ((Holder)args[1]).Value == 42 ? 42 : 1;
    }
}
