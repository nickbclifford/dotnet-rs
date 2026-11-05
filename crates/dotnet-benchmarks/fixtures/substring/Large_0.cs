using System;
public class Program {
    public static int Main() {
        char[] chars = new char[200000];
        string source = new string(chars);
        int total = 0;
        for (int i = 0; i < 80; i++) {
            string result = source.Substring(i, 150000);
            total += result.Length;
        }
        return total == 12000000 ? 0 : 1;
    }
}
