using System;
using System.Reflection;
using System.Threading;

class JoinRaceTarget
{
    public int Field;
    public int Property { get; set; }
    public void Method() { }
}
class FinalizerRequestTarget
{
    public int Value { get; set; }
    public void Touch() { }
}
sealed class FinalizerSentinel
{
    ~FinalizerSentinel() { Program.WaitFromFinalizer(); }
}

static class Program
{
    static volatile bool start;
    static int ready;
    static volatile bool finished;
    static readonly MethodInfo?[] methods = new MethodInfo?[3];
    static readonly FieldInfo?[] fields = new FieldInfo?[3];
    static readonly PropertyInfo?[] properties = new PropertyInfo?[3];
    static Thread? requester;
    static volatile bool finalizerEntered;
    static PropertyInfo? finalizerProperty;
    static bool finalizerMethodFound;

    static void Race(int index)
    {
        Interlocked.Increment(ref ready);
        while (!start) Thread.Sleep(0);
        Type target = typeof(JoinRaceTarget);
        methods[index] = target.GetMethod("Method");
        fields[index] = target.GetField("Field");
        properties[index] = target.GetProperty("Property");
    }

    internal static void WaitFromFinalizer()
    {
        finalizerEntered = true;
        requester!.Join();
    }

    static void CollectWhileRequestsRun()
    {
        while (!finished)
        {
            GC.Collect();
            Thread.Sleep(0);
        }
    }

    static void RequestFromFinalizerWait()
    {
        while (!finalizerEntered) Thread.Sleep(0);
        Type target = typeof(FinalizerRequestTarget);
        finalizerProperty = target.GetProperty("Value");
        finalizerMethodFound = target.GetMethod("Touch") != null;
    }

    static void CreateFinalizerSentinel() { _ = new FinalizerSentinel(); }

    static int Main()
    {
        var a = new Thread(() => Race(0));
        var b = new Thread(() => Race(1));
        var c = new Thread(() => Race(2));
        a.Start(); b.Start(); c.Start();
        while (Volatile.Read(ref ready) != 3) Thread.Sleep(0);
        var collector = new Thread(CollectWhileRequestsRun);
        collector.Start();
        start = true;
        // Main remains in Join while workers miss the same canonical tables and another executor
        // repeatedly requests collections; their rooted operands stay live across service waits.
        a.Join(); b.Join(); c.Join();
        finished = true;
        collector.Join();
        bool raceOk = methods[0] != null && fields[0] != null && properties[0] != null
            && ReferenceEquals(methods[0], methods[1]) && ReferenceEquals(methods[0], methods[2])
            && ReferenceEquals(fields[0], fields[1]) && ReferenceEquals(fields[0], fields[2])
            && ReferenceEquals(properties[0], properties[1]) && ReferenceEquals(properties[0], properties[2]);

        requester = new Thread(RequestFromFinalizerWait);
        requester.Start();
        CreateFinalizerSentinel();
        GC.Collect();
        // The finalizer joins the requesting worker; its idle native wait must still service the
        // worker's property and method misses before the Join returns.
        GC.WaitForPendingFinalizers();
        requester.Join();
        GC.Collect();
        bool ok = raceOk && finalizerMethodFound && finalizerProperty != null
            && finalizerProperty.Name == "Value"
            && ReferenceEquals(finalizerProperty, typeof(FinalizerRequestTarget).GetProperty("Value"));
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
