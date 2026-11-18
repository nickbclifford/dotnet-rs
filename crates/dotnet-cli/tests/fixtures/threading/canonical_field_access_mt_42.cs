using System;
using System.Reflection;
using System.Threading;

class FieldValue { }
class FieldOwner { public FieldValue Value = new FieldValue(); }
class GenericOwner<T> { public T? Value; }
static class Program
{
    static FieldInfo field = typeof(FieldOwner).GetField("Value")!;
    static FieldInfo genericField = typeof(GenericOwner<int>).GetField("Value")!;
    static Type? first, second, declaring, genericDeclaring;
    static volatile bool go;
    static int ready;
    static void Run(bool left)
    {
        Interlocked.Increment(ref ready);
        while (!go) Thread.Sleep(0);
        var result = field.FieldType;
        if (left) { first = result; declaring = field.DeclaringType; genericDeclaring = genericField.DeclaringType; }
        else second = result;
    }
    static int Main()
    {
        var a = new Thread(() => Run(true));
        var b = new Thread(() => Run(false));
        a.Start(); b.Start();
        while (Volatile.Read(ref ready) != 2) Thread.Sleep(0);
        go = true;
        a.Join(); b.Join();
        GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();
        bool ok = ReferenceEquals(first, second) && ReferenceEquals(first, typeof(FieldValue))
            && ReferenceEquals(declaring, typeof(FieldOwner)) && genericDeclaring != null;
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
