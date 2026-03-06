using System;

public class Program
{
    public static int Main()
    {
        // 1. Integer spans (fast path)
        int[] arr1 = new int[3];
        arr1[0] = 1; arr1[1] = 2; arr1[2] = 3;
        int[] arr2 = new int[3];
        arr2[0] = 1; arr2[1] = 2; arr2[2] = 3;
        int[] arr3 = new int[3];
        arr3[0] = 1; arr3[1] = 2; arr3[2] = 4;
        
        ReadOnlySpan<int> s1 = arr1;
        ReadOnlySpan<int> s2 = arr2;
        ReadOnlySpan<int> s3 = arr3;
        
        if (!s1.SequenceEqual(s2)) return 1;
        if (s1.SequenceEqual(s3)) return 2;
        
        // 2. String spans (slow path, string implements IEquatable<string>)
        string[] sa1 = new string[2];
        sa1[0] = "a"; sa1[1] = "b";
        string[] sa2 = new string[2];
        sa2[0] = "a"; sa2[1] = "b";
        string[] sa3 = new string[2];
        sa3[0] = "a"; sa3[1] = "c";
        
        ReadOnlySpan<string> ss1 = sa1;
        ReadOnlySpan<string> ss2 = sa2;
        ReadOnlySpan<string> ss3 = sa3;
        
        if (!ss1.SequenceEqual(ss2)) return 3;
        if (ss1.SequenceEqual(ss3)) return 4;

        // 3. Different lengths
        int[] arr4 = new int[2];
        arr4[0] = 1; arr4[1] = 2;
        ReadOnlySpan<int> s4 = arr4;
        if (s1.SequenceEqual(s4)) return 5;
        
        // 4. Empty spans
        ReadOnlySpan<int> sEmpty1 = ReadOnlySpan<int>.Empty;
        ReadOnlySpan<int> sEmpty2 = new int[0];
        if (!sEmpty1.SequenceEqual(sEmpty2)) return 6;

        return 0;
    }
}
