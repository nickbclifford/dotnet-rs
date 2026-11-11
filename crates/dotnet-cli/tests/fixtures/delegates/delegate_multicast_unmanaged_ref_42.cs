using System;

public delegate void Update(ref int value);
public static class Program {
    static void First(ref int value) { value += 10; }
    static void Second(ref int value) { value += 30; }
    public static unsafe int Main() {
        int number = 2;
        int* pointer = &number;
        Update update = First;
        update += Second;
        update(ref *pointer);
        Console.WriteLine(number);
        return number == 42 ? 42 : 1;
    }
}
