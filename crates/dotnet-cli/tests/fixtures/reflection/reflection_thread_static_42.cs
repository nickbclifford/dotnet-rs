using System;
using System.Reflection;
using System.Threading;

public class Slots<T> { [ThreadStatic] public static int Value; public static object Shared; }
public class Payload { public string Text; }
public static class Program
{
    public static int Main()
    {
        FieldInfo field = typeof(Slots<int>).GetField("Value")!;
        field.SetValue(null, 17);
        int before = -1, after = -1;
        var thread = new Thread(() => {
            before = (int)field.GetValue(null)!;
            field.SetValue(null, 29);
            after = Slots<int>.Value;
            typeof(Slots<int>).GetField("Shared")!.SetValue(null, new Payload { Text = new string('x', 4) });
        });
        thread.Start(); thread.Join();
        GC.Collect();
        var shared = (Payload)typeof(Slots<int>).GetField("Shared")!.GetValue(null)!;
        Console.WriteLine($"{before}:{after}:{field.GetValue(null)}:{Slots<int>.Value}:{shared.Text}");
        return 42;
    }
}
