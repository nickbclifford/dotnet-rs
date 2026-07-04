using System;

interface IBaseType
{
    int GetValue() => throw new NotSupportedException();
}

interface IDerivedType : IBaseType
{
    int IBaseType.GetValue() => 42;
}

sealed class Impl : IDerivedType
{
}

public static class Program
{
    public static int Main()
    {
        IBaseType value = new Impl();
        return value.GetValue();
    }
}
