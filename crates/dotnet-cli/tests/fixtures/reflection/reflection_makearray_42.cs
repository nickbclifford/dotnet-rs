using System;

public sealed class ArrayProbe
{
}

public static class Program
{
    public static int Main()
    {
        Type arrayType = typeof(ArrayProbe).MakeArrayType();

        if (!arrayType.IsArray)
        {
            return 1;
        }

        if (arrayType.GetElementType() != typeof(ArrayProbe))
        {
            return 2;
        }

        if (typeof(ArrayProbe).MakeArrayType() != arrayType)
        {
            return 3;
        }

        return 42;
    }
}
