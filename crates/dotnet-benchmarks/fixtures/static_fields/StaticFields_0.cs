public static class StaticCell {
    public static int Value;
}

public class Program {
    public static int Main() {
        const int Iterations = 30_000;
        StaticCell.Value = 1;
        int sum = 0;
        for (int i = 0; i < Iterations; i++) {
            sum += StaticCell.Value;
            StaticCell.Value = i;
            ref int cell = ref StaticCell.Value;
            cell += 1;
        }

        return sum == 449_985_001 && StaticCell.Value == Iterations ? 0 : 1;
    }
}
