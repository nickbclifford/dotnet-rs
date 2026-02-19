using System;

public class Program
{
    public static int Main()
    {
        string s = "hello";
        ReadOnlySpan<char> span = s; // This calls op_Implicit
        if (span.Length != 5) return 1;
        if (span[0] != 'h') return 2;
        if (span[4] != 'o') return 3;
        
        string empty = "";
        ReadOnlySpan<char> emptySpan = empty;
        if (emptySpan.Length != 0) return 4;
        
        string @null = null;
        ReadOnlySpan<char> nullSpan = @null;
        if (nullSpan.Length != 0) return 5;

        return 0;
    }
}
