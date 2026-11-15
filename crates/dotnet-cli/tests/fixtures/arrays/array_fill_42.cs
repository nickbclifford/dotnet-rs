using System;

public static class Program
{
    private sealed class Marker { }

    private struct Payload
    {
        public int Number;
        public object? Reference;

        public Payload(int number, object? reference)
        {
            Number = number;
            Reference = reference;
        }
    }

    private static bool ThrowsExactly<TException>(Action action, string? paramName)
        where TException : Exception
    {
        try
        {
            action();
        }
        catch (Exception exception)
        {
            if (exception.GetType() != typeof(TException)) return false;
            return exception is not ArgumentException argument || argument.ParamName == paramName;
        }

        return false;
    }

    public static int Main()
    {
        int[] values = { 1, 2, 3, 4, 5, 6 };
        Array.Fill(values, 9);
        for (int i = 0; i < values.Length; i++)
        {
            if (values[i] != 9) return 1;
        }

        Array.Fill(values, 4, 2, 3);
        if (values[0] != 9 || values[1] != 9 || values[2] != 4 || values[3] != 4 ||
            values[4] != 4 || values[5] != 9)
            return 2;

        Array.Fill(values, 7, values.Length, 0);
        int[] empty = Array.Empty<int>();
        Array.Fill(empty, 10);
        Array.Fill(empty, 11, 0, 0);

        var marker = new Marker();
        var replacement = new Marker();
        Marker?[] references = new Marker?[4];
        Array.Fill(references, marker);
        Array.Fill(references, replacement, 1, 2);
        if (!ReferenceEquals(references[0], marker) || !ReferenceEquals(references[1], replacement) ||
            !ReferenceEquals(references[2], replacement) || !ReferenceEquals(references[3], marker))
            return 3;

        Payload[] payloads = new Payload[4];
        Array.Fill(payloads, new Payload(17, marker));
        Array.Fill(payloads, new Payload(23, replacement), 1, 2);
        if (payloads[0].Number != 17 || !ReferenceEquals(payloads[0].Reference, marker) ||
            payloads[1].Number != 23 || !ReferenceEquals(payloads[1].Reference, replacement) ||
            payloads[2].Number != 23 || !ReferenceEquals(payloads[2].Reference, replacement) ||
            payloads[3].Number != 17 || !ReferenceEquals(payloads[3].Reference, marker))
            return 4;

        int[]? nullArray = null;
        if (!ThrowsExactly<ArgumentNullException>(() => Array.Fill(nullArray!, 0), "array")) return 5;
        if (!ThrowsExactly<ArgumentNullException>(() => Array.Fill(nullArray!, 0, -1, -1), "array")) return 6;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Array.Fill(values, 0, -1, 0), "startIndex")) return 7;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Array.Fill(values, 0, values.Length + 1, 0), "startIndex")) return 8;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Array.Fill(values, 0, 0, -1), "count")) return 9;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Array.Fill(values, 0, 4, 3), "count")) return 10;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Array.Fill(values, 0, 1, int.MaxValue), "count")) return 11;

        return 42;
    }
}
