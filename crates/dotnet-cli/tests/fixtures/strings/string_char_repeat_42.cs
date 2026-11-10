using System;

public class Program
{
    public static int Main()
    {
        string s = new string('x', 5);
        if (s.Length != 5 || s != "xxxxx") return 1;
        if (new string('y', 0) != "") return 2;
        try
        {
            new string('z', -1);
            return 3;
        }
        catch (ArgumentOutOfRangeException) { }
        return 42;
    }
}
