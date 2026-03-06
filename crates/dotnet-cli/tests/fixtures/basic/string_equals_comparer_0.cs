using System;
using System.Collections.Generic;

public class Program
{
    public static int Main()
    {
        // Test: use EqualityComparer<string>.Default.Equals
        string s1 = "a";
        string s2 = "a";
        string s3 = "b";

        if (!EqualityComparer<string>.Default.Equals(s1, s2)) return 1;
        if (EqualityComparer<string>.Default.Equals(s1, s3)) return 2;

        return 0;
    }
}
