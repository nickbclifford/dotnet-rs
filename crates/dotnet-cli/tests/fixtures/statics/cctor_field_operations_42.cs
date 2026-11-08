using System;

public struct Payload { public object Reference; public int Number; }
public class StoreTarget
{
    public static Payload Value;
    static StoreTarget() { GC.Collect(); }
}
public class AddressTarget
{
    public static int Value;
    static AddressTarget() { Value = 10; }
}
public class FailedStore
{
    public static object Value;
    static FailedStore() { throw new InvalidOperationException(); }
}
public class Program
{
    static void Increment(ref int value) { value++; }
    public static int Main()
    {
        object marker = new object();
        StoreTarget.Value = new Payload { Reference = marker, Number = 31 };
        Increment(ref AddressTarget.Value);
        try { FailedStore.Value = marker; return 1; }
        catch (TypeInitializationException ex)
        {
            if (!(ex.InnerException is InvalidOperationException)) return 2;
        }
        return object.ReferenceEquals(StoreTarget.Value.Reference, marker) &&
            StoreTarget.Value.Number + AddressTarget.Value == 42 ? 42 : 3;
    }
}
