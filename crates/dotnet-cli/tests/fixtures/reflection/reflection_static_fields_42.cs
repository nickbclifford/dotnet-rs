using System;
using System.Reflection;

public struct Data { public int Number; public string Text; }
public static class Log { public static int Order; }
public unsafe class Fields
{
    public static int Number;
    public static Data Struct;
    public static object Reference;
    public static int* Pointer;
    public static readonly int ReadOnly;
    public const int Literal = 9;
    static Fields() { Log.Order = Log.Order * 10 + 1; Number = 7; ReadOnly = 11; }
}
public class WriteFirst
{
    public static int Number;
    static WriteFirst() { Log.Order = Log.Order * 10 + 2; Number = 4; }
}
public class Failure
{
    public static int Number;
    static Failure() { Log.Order = Log.Order * 10 + 3; throw new InvalidOperationException(); }
}
public class Reentrant
{
    public static int Number;
    public static readonly int ReadOnly;
    static Reentrant()
    {
        var number = typeof(Reentrant).GetField("Number")!;
        Console.WriteLine($"recursive:{number.GetValue(null)}");
        number.SetValue(null, 14);
        try { typeof(Reentrant).GetField("ReadOnly")!.SetValue(null, 15); Console.WriteLine("recursive readonly writable"); }
        catch (FieldAccessException) { Console.WriteLine("recursive readonly denied"); }
    }
}
public class Indirect
{
    public static int Value;
    static Indirect() { Value = 77; }
}
public class IndirectSecond
{
    public static int Value;
    static IndirectSecond() { Value = 88; }
}
public class Retained
{
    public static object Value;
    static Retained() { GC.Collect(); }
}
public class ReadOnlyFirst
{
    public static readonly int Value;
    static ReadOnlyFirst() { Log.Order = Log.Order * 10 + 4; Value = 4; }
}
public class Generic<T> { public static T Value; public static int Number; [ThreadStatic] public static int ThreadNumber; }
public static unsafe class Program
{
    static FieldInfo Field(Type t, string name) => t.GetField(name)!;
    public static int Main()
    {
        var number = Field(typeof(Fields), "Number");
        Console.WriteLine($"{Log.Order}:{number.GetValue(new object())}:{Log.Order}");
        number.SetValue(null, 19);
        Console.WriteLine(Fields.Number);
        var indirect = Field(typeof(Indirect), "Value");
        Func<object, object> read = indirect.GetValue;
        Console.WriteLine($"indirect:{read(null!)}");
        var method = typeof(FieldInfo).GetMethod("GetValue")!;
        var second = Field(typeof(IndirectSecond), "Value");
        Console.WriteLine($"invoke:{method.Invoke(second, new object[] { null! })}");
        Func<object, object> multicast = read + second.GetValue;
        Console.WriteLine($"multicast:{multicast(null!)}");
        var data = Field(typeof(Fields), "Struct");
        data.SetValue(null, new Data { Number = 12, Text = "hello" });
        Data got = (Data)data.GetValue(null)!;
        Console.WriteLine($"{got.Number}:{got.Text}");
        data.SetValue(null, null);
        Console.WriteLine(((Data)data.GetValue(null)!).Number);
        var reference = Field(typeof(Fields), "Reference");
        reference.SetValue(null, "kept");
        GC.Collect();
        Console.WriteLine(reference.GetValue(null));
        int local = 3;
        var pointer = Field(typeof(Fields), "Pointer");
        pointer.SetValue(null, Pointer.Box(&local, typeof(int*)));
        Console.WriteLine(Pointer.Unbox(pointer.GetValue(null)!) == &local && Fields.Pointer == &local);
        Console.WriteLine(Field(typeof(Fields), "Literal").GetValue(null));
        try { Field(typeof(Fields), "Literal").SetValue(null, 10); } catch (FieldAccessException) { Console.WriteLine("literal"); }
        try { Field(typeof(Fields), "ReadOnly").SetValue(null, 5); }
        catch (FieldAccessException) { Console.WriteLine("readonly"); }
        var first = Field(typeof(WriteFirst), "Number");
        first.SetValue(null, 6);
        Console.WriteLine($"{Log.Order}:{WriteFirst.Number}");
        var failure = Field(typeof(Failure), "Number");
        for (int i = 0; i < 2; i++)
            try { failure.GetValue(null); } catch (TargetInvocationException e) { Console.WriteLine(e.InnerException is TypeInitializationException tie && tie.InnerException is InvalidOperationException); }
        Console.WriteLine(Log.Order);
        try { failure.SetValue(null, 3); } catch (TargetInvocationException e) { Console.WriteLine(e.InnerException is TypeInitializationException); }
        try { Field(typeof(ReadOnlyFirst), "Value").SetValue(null, 5); } catch (FieldAccessException) { Console.WriteLine($"readonly first:{Log.Order}"); }
        Console.WriteLine($"reentrant:{Field(typeof(Reentrant), "Number").GetValue(null)}");
        Field(typeof(Retained), "Value").SetValue(null, new Data { Number = 87, Text = "root" });
        var retained = (Data)Retained.Value;
        Console.WriteLine($"retained:{retained.Number}:{retained.Text}");
        var gi = Field(typeof(Generic<int>), "Number");
        var gs = Field(typeof(Generic<string>), "Number");
        gi.SetValue(null, 21); gs.SetValue(null, 22);
        Console.WriteLine($"{Generic<int>.Number}:{Generic<string>.Number}:{gi.GetValue(null)}:{gs.GetValue(null)}");
        Field(typeof(Generic<int>), "Value").SetValue(null, 41);
        Field(typeof(Generic<string>), "Value").SetValue(null, "generic");
        Console.WriteLine($"{Generic<int>.Value}:{Generic<string>.Value}");
        var thread = Field(typeof(Generic<int>), "ThreadNumber");
        thread.SetValue(null, 13);
        Console.WriteLine($"{thread.GetValue(null)}:{Generic<int>.ThreadNumber}");
        return 42;
    }
}
