using System;
using System.Collections.Generic;
using System.Linq;

enum State : int { One = 1, Two = 2 }
enum Other : uint { One = 1 }

class Program
{
    static T Cast<T>(object value) => (T)value;
    static bool Is<T>(object value) => value is T;
    static int Main()
    {
        int[][] jagged = { new[] { 1, 2 }, new[] { 3 } };
        var list = jagged.ToList();
        State[][] states = { new[] { State.One, State.Two } };
        var stateList = new List<State[]>(states);
        var indexed = (IList<int[]>)(object)jagged;
        var generic = Cast<int[]>(jagged[0]);
        int sum = 0;
        foreach (int[] item in (IEnumerable<int[]>)(object)jagged)
            foreach (int value in item) sum += value;
        var ints = (int[])(object)states[0];
        var unsigned = (uint[])(object)generic;
        Console.WriteLine($"{list.Count} {stateList.Count} {indexed[1][0]} {generic[1]} {sum} {ints[0]} {unsigned[1]}");
        Console.WriteLine($"{Is<int[]>(states[0])} {Is<uint[]>(generic)} {Is<Other[]>(states[0])} {Is<long[]>(generic)} {Is<object[]>(generic)}");
        Console.WriteLine($"{Is<IList<uint>>(generic)} {Is<IList<int>>(states[0])}");
        Console.WriteLine($"{Cast<uint[]>(generic)[0]} {Cast<int[]>(states[0])[1]}");
        try { Cast<long[]>(generic); Console.WriteLine("unexpected cast"); }
        catch (InvalidCastException) { Console.WriteLine("invalid cast"); }
        return 42;
    }
}
