public struct Pair
{
    public int A;
    public int B;
}

public static class Program
{
    public static int Main()
    {
        Pair[] arr = new Pair[3];
        arr[0] = new Pair { A = 10, B = 11 };
        arr[1] = new Pair { A = 20, B = 21 };
        arr[2] = new Pair { A = 30, B = 31 };

        ref Pair p1 = ref arr[1];
        if (p1.A != 20) return 1;
        if (p1.B != 21) return 2;
        if (arr[2].A != 30) return 3;
        if (arr[2].B != 31) return 4;
        return 42;
    }
}
