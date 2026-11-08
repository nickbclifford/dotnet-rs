using System;
using System.Reflection;

public class ReferenceThing
{
    public int Value;
    public ReferenceThing(int value) { GC.Collect(); Value = value; }
}
public sealed class Token
{
    public int Number;
}
public struct ValueThing
{
    public object Value;
    public int Number;
    public ValueThing(int number)
    {
        Value = new Token { Number = number };
        Number = number;
        GC.Collect();
    }
}
public struct GenericThing<T>
{
    public T Value;
    public int Number;
    public GenericThing(T value) { Value = value; Number = 42; GC.Collect(); }
}
public static class Program
{
    public static int Main()
    {
        var cls = typeof(ReferenceThing).GetConstructor(new[] { typeof(int) })!;
        var val = typeof(ValueThing).GetConstructor(new[] { typeof(int) })!;
        var x = (ReferenceThing)cls.Invoke(new object[] { 42 });
        var y = (ValueThing)val.Invoke(new object[] { 42 });
        var z = (ReferenceThing)Activator.CreateInstance(typeof(ReferenceThing), new object[] { 42 })!;
        var w = (ValueThing)Activator.CreateInstance(typeof(ValueThing), new object[] { 42 })!;
        var generic = (GenericThing<string>)Activator.CreateInstance(typeof(GenericThing<string>), new object[] { "generic" })!;
        object? nullable = Activator.CreateInstance(typeof(int?), new object[] { 42 });
        var nullableCtor = typeof(int?).GetConstructor(new[] { typeof(int) })!;
        object? nullableViaCtor = nullableCtor.Invoke(new object[] { 42 });
        return x.Value == 42 && y.Number == 42 && ((Token)y.Value).Number == 42
            && z.Value == 42 && w.Number == 42 && ((Token)w.Value).Number == 42
            && generic.Value == "generic" && generic.Number == 42
            && nullable is int i && i == 42 && nullableViaCtor is int j && j == 42 ? 42 : 1;
    }
}
