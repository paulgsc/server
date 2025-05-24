
---

## **Definitions**

Let:

* Let $A = [(k_1, w_1), (k_2, w_2), \ldots, (k_n, w_n)]$ be the input array of tuples, where:

  * $k_i$ is a unique identifier (key),
  * $w_i \in \mathbb{R}$ is a weight.
* Let $W = \text{sorted}(\{w_i\}, \text{descending}) = [w^{(0)}, w^{(1)}, \ldots, w^{(L-1)}]$ be the set of **distinct** weight values in **descending** order (for max-heap mode; min-heap would reverse this).
* Let $L$ be the number of unique weight levels.
* Let layer $\ell_j$ (layer index from **top = 0** to **bottom = L - 1**) represent the set:

  $$
  \ell_j = \{ k_i \mid w_i = w^{(j)} \}
  $$
* Let $|\ell_j|$ denote the number of elements in layer $j$.

---

## **Plane/Canvas Geometry**

We will lay this out on a 2D grid $\mathbb{Z}^2$, where each brick occupies a unit width and unit height.

Let $M = \max_j |\ell_j|$ be the width of the **widest layer**. Then the layout is placed in a virtual grid of dimensions:

$$
\text{Width} = M, \quad \text{Height} = L
$$

This defines the bounding box or "canvas".

---

## **Constraints**

### **C1: Hierarchical Support Constraint**

Each layer $\ell_j$ (with $j > 0$) must be **supported** by the layer $\ell_{j+1}$ below:

$$
|\ell_{j+1}| \geq |\ell_j|
$$

### **C2: Alignment Modes**

Two modes of layout are supported.

#### (a) **Pyramid Mode**

* Each brick in layer $\ell_j$ is placed such that its center aligns with the **midpoint** between two bricks in the layer $\ell_{j+1}$ below it.
* More formally, for a layer $\ell_j$ of width $m$, and a lower layer $\ell_{j+1}$ of width $n$, the starting x-index of layer $\ell_j$ is:

  $$
  x^{(j)} = \left\lfloor \frac{n - m}{2} \right\rfloor
  $$

  Then:

  $$
  \text{Positions in layer } \ell_j: \{(x^{(j)} + i, j) \mid i = 0, 1, \ldots, m - 1\}
  $$

#### (b) **Step Wall (Right-Aligned) Mode**

* Each layer is aligned to the **rightmost edge** of the grid width $M$.
* Then:

  $$
  x^{(j)} = M - |\ell_j|
  $$

  and:

  $$
  \text{Positions in layer } \ell_j: \{(x^{(j)} + i, j) \mid i = 0, 1, \ldots, |\ell_j| - 1\}
  $$

---

## **Worst Case Geometry Observation**

* If all weights are the same ($L = 1$), all bricks are on the bottom layer.
* So the bounding box has:

  $$
  \text{Width} = N, \quad \text{Height} = 1
  $$

Thus, in the worst case, the plane is $N \times 1$. All permutations will fit in a bounding plane of size $N \times N$ (for generality), but most use far less height due to weight stratification.

---

## **Additional Notes**

* Layers are always arranged in decreasing (or increasing, for min-mode) weight order.
* For visual clarity, layers can optionally be color-coded by their weight levels.
* You can optionally enforce **horizontal centering** of entire layout in the canvas by padding the left/right.

---

