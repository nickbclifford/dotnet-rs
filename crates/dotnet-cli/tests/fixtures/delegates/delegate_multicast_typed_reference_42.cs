using System;

public delegate void Update(TypedReference value);
public static class Program {
    static void First(TypedReference value) { __refvalue(value, int) += 10; }
    static void Second(TypedReference value) { __refvalue(value, int) += 30; }
    public static int Main() {
        int number = 2;
        Update update = First;
        update += Second;
        update(__makeref(number));
        Console.WriteLine(number);
        return number == 42 ? 42 : 1;
    }
}
