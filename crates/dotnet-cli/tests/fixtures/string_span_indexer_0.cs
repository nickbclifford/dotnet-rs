using System;

public class Program
{
    public static int Main()
    {
        // Test: access string array elements through ReadOnlySpan indexer
        string[] arr = new string[2];
        arr[0] = "a";
        arr[1] = "b";

        ReadOnlySpan<string> span = arr;

        string s0 = span[0];
        string s1 = span[1];

        if (s0 != "a") return 1;
        if (s1 != "b") return 2;

        return 0;
    }
}
