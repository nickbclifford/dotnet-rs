using System;

public enum Choice : long { Low = 1, High = 42 }
public delegate Choice Pick(Choice value);
public class Program {
    static Choice First(Choice value) { Console.WriteLine((long)value); return Choice.Low; }
    static Choice Second(Choice value) { Console.WriteLine((long)value); return Choice.High; }
    public static int Main() {
        Pick pick = First;
        pick += Second;
        return (int)pick(Choice.High);
    }
}
