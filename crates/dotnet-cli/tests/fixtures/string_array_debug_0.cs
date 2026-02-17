using System;

public class Program
{
    public static int Main()
    {
        // Create string array
        string[] arr = new string[2];
        arr[0] = "hello";
        arr[1] = "world";

        // Load element directly
        string s0 = arr[0];
        string s1 = arr[1];

        // Compare directly
        if (s0 != "hello") return 1;
        if (s1 != "world") return 2;
        if (!s0.Equals("hello")) return 3;
        if (!s1.Equals("world")) return 4;

        // Create span from array
        ReadOnlySpan<string> span = arr;

        // Access via span indexer
        string spanS0 = span[0];
        string spanS1 = span[1];

        // Compare span-loaded strings
        if (spanS0 != "hello") return 5;
        if (spanS1 != "world") return 6;
        if (!spanS0.Equals("hello")) return 7;
        if (!spanS1.Equals("world")) return 8;

        // Compare span-loaded vs array-loaded
        if (spanS0 != s0) return 9;
        if (spanS1 != s1) return 10;

        return 0;
    }
}
