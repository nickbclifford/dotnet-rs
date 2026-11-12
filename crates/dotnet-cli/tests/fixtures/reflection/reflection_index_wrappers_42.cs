using System;
using System.Reflection;

public sealed class Sample
{
    public int Value;
    public Sample(int value) { Value = value; }
    public int Add(int value) { return Value + value; }
}

public static class Program
{
    public static int Main()
    {
        Type type = typeof(Sample);
        if (type != typeof(Sample)) return 1;
        ConstructorInfo constructor = type.GetConstructor(new[] { typeof(int) })!;
        MethodInfo method = type.GetMethod("Add")!;
        FieldInfo field = type.GetField("Value")!;
        if (constructor == null || method == null || field == null) return 2;
        Sample sample = (Sample)constructor.Invoke(new object[] { 20 });
        if ((int)field.GetValue(sample)! != 20) return 3;
        field.SetValue(sample, 22);
        if ((int)method.Invoke(sample, new object[] { 20 })! != 42) return 4;
        if (field.FieldType != typeof(int) || method.ReturnType != typeof(int)) return 5;
        return 42;
    }
}
