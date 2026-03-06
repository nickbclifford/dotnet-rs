using System;

public class Program
{
    public static int Main()
    {
        // Simple test: create string array and read elements
        string[] arr = new string[2];
        arr[0] = "a";
        arr[1] = "b";

        string s0 = arr[0];
        string s1 = arr[1];

        if (s0 != "a") return 1;
        if (s1 != "b") return 2;

        return 0;
    }
}
