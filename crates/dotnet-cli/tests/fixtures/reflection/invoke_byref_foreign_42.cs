using System;
using System.Threading;

public static class Program
{
    static object[]? shared;
    static volatile bool ready;
    static volatile bool finished;

    static void Owner()
    {
        shared = new object[] { 40, "before", null! };
        ready = true;
        while (!finished) Thread.Sleep(0);
    }

    public static void Update(ref int number, ref object value, out int created)
    {
        GC.Collect();
        number += 2;
        value = "after";
        created = 40;
    }

    public static int Main()
    {
        var owner = new Thread(Owner);
        owner.Start();
        while (!ready) Thread.Sleep(0);
        try
        {
            object[] args = shared!;
            shared = null;
            typeof(Program).GetMethod(nameof(Update))!.Invoke(null, args);
            return (int)args[0] == 42 && (string)args[1] == "after" && (int)args[2] == 40 ? 42 : 1;
        }
        finally
        {
            finished = true;
            owner.Join();
        }
    }
}
