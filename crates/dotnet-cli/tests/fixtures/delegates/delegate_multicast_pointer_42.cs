using System;
public unsafe delegate int* PointerAction(int* value);
public unsafe delegate delegate*<int, int> FunctionAction(delegate*<int, int> value);
public static unsafe class Program
{
    static int order;
    static int* First(int* value) { order = 1; *value += 10; GC.Collect(); return value + 1; }
    static int* Last(int* value) { order = order * 10 + 2; *value += 30; return value; }
    static int AddOne(int x) => x + 1;
    static int AddTwo(int x) => x + 2;
    static delegate*<int, int> OtherFunction(delegate*<int, int> value) => &AddOne;
    static delegate*<int, int> SameFunction(delegate*<int, int> value) { GC.Collect(); return value; }
    public static int Main()
    {
        int* value = stackalloc int[2]; value[0] = 2; value[1] = 9;
        PointerAction action = First; action += Last;
        int* result = action(value); *result += 0;
        FunctionAction functions = OtherFunction; functions += SameFunction;
        delegate*<int, int> function = &AddTwo;
        delegate*<int, int> selected = functions(function);
        int answer = selected(40);
        Console.WriteLine($"{order} {value[0]} {value[1]} {answer}");
        return result == value && order == 12 && value[0] == 42 && value[1] == 9 && answer == 42 ? 42 : 1;
    }
}
