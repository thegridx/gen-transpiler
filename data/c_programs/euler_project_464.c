/*
  https://projecteuler.net/problem=464

  Mobius Function and Intervals
  Problem 464

  .....
  Let P(a,b) be the number of integers in the interval [a,b] such that mu(n) = 1
  Let N(a,b) be the number of integers in the interval [a,b] such that mu(n) = -1
  For example  P(2, 10) = 2
  For example  N(2, 10) = 4

  Let C(n) be the number of (a,b) such that:
  * a <= a <= b <= n
  * 99 N(a,b) <= 100 P(a,b)
  * 99 P(a,b) <= 100 N(a,b)

  .....
  Find C(20 000 000)
*/



EP464_INT pivot_split(EP464_INT l, EP464_INT r, EP464_INT element_sz, EP464_INT offset, EP464_INT pivot, EP464_INT *x) {
  EP464_INT i = l, j = r;
  --l;
  while(1) {
    while(i < r && x[element_sz * (--r) + offset] >= pivot);
    while(j - 1 > l && x[element_sz * (++l) + offset] < pivot);
    if(l >= r) break;
    // swap
    EP464_INT i = 0;
    EP464_INT tmp;
    for (i = 0; i < element_sz; ++i) {
      tmp = x[element_sz * l + i];
      x[element_sz * l + i] = x[element_sz * r + i];
      x[element_sz * r + i] = tmp;
    }
  }
  ++r;
  if (x[element_sz * (r - 1) + offset] >= pivot)
    --r;
  return r; // left of here, the array is < then the pivot
}

EP464_INT inner(EP464_INT l1, EP464_INT r1, EP464_INT l2, EP464_INT r2, EP464_INT offset, EP464_INT *x) {
  EP464_INT mx1 = x[(l1 << 1) + offset], mn1 = mx1;
  EP464_INT mx2 = x[(l2 << 1) + offset], mn2 = mx2;
  EP464_INT i;
  for (i = l1 + 1; i < r1; ++i) {
    EP464_INT v = x[(i << 1) + offset];
    if (v > mx1) mx1 = v;
    if (v < mn1) mn1 = v;
  }
  for (i = l2 + 1; i < r2; ++i) {
    EP464_INT v = x[(i << 1) + offset];
    if (v > mx2) mx2 = v;
    if (v < mn2) mn2 = v;
  }
  if (mx1 <= mn2) { // all pairs are good in this offset
    return offset == 0 ? inner(l1, r1, l2, r2, 1, x) : (r1 -l1) * (r2 - l2);
  } else if (mx2 < mn1) { // no pair is good in this offset
    return 0;
  } else if (mx1 == mn1) {
    EP464_INT m = pivot_split(l2, r2, 2, offset, mx1, x);
    return offset == 0 ? inner(l1, r1, m, r2, 1, x) : (r1 -l1) * (r2 - m);
  } else if (mx2 == mn2) {
    EP464_INT m = pivot_split(l1, r1, 2, offset, mx2 + 1, x);
    return offset == 0 ? inner(l1, m, l2, r2, 1, x) : (m -l1) * (r2 - l2);
  }
  EP464_INT pivot = ((mn1 > mn2 ? mn1 : mn2) + (mx1 < mx2 ? mx1 : mx2)) >> 1;
  EP464_INT m1 = pivot_split(l1, r1, 2, offset, pivot + 1, x);
  EP464_INT m2 = pivot_split(l2, r2, 2, offset, pivot + 1, x);
  EP464_INT ret = 0;
  if (m1 > l1 && m2 > l2)
    ret += inner(l1, m1, l2, m2, offset, x);
  if (r1 > m1 && r2 > m2)
    ret += inner(m1, r1, m2, r2, offset, x);
  if (m1 > l1 && r2 > m2)
    ret += offset == 0 ? inner(l1, m1, m2, r2, 1, x) : (m1 -l1) * (r2 - m2);
  return ret;
}

EP464_INT outer(EP464_INT left, EP464_INT right, EP464_INT* x) {
  EP464_INT sz = right - left;
  EP464_INT ret = 0;
  if (sz == 1) {
    ret = x[2 * left] >= 0 && x[2 * left + 1] >= 0;
    return ret;
  }
  EP464_INT half = sz >> 1;
  if (half > 0)
    ret += outer(left, left + half, x);
  if (right > left + half)
    ret += outer(left + half, right, x);
  if (half > 0 && right > left + half) 
    ret += inner(left, left + half, left + half, right, 0, x);
  return ret;
}

EP464_INT bit_array_size(EP464_INT k) {
  EP464_INT k_size = sizeof(k);
  return ((k >> 3) + k_size) & (~(k_size - (EP464_INT)(1)));
}

EP464_INT work(EP464_INT k, char *all_mem) {
  EP464_INT i, j;
  EP464_INT bit_array_sz = bit_array_size(k);
  char *is_one_or_square_free = all_mem;
  char *num_factors_mod_2 = is_one_or_square_free + bit_array_sz;
  char *prime_sieve = num_factors_mod_2 + bit_array_sz;
  for (EP464_INT i = 0; i <= (k >> 3); ++i) {
    is_one_or_square_free[i] = 0xFF;
    num_factors_mod_2[i] = 0;
    prime_sieve[i] = 0xFF;
  }
  is_one_or_square_free[0] = 0xFE;
  EP464_INT step;
  for (i = 2; i <= k; ++i) {
    EP464_INT i_0 = i >> 3;
    char i_1 = 1 << (i & 7);
    if (prime_sieve[i_0] & i_1) {
      for (j = 2 * i; j <= k; j += i) {
	EP464_INT j_0 = j >> 3; 
	char j_1 = 1 << (j & 7);
	prime_sieve[j_0] &= ~j_1;
	num_factors_mod_2[j_0] ^= j_1;
      }
      num_factors_mod_2[i_0] ^= i_1;
      if (i < 4500) {
	step = i * i;
	for (j = step; j <= k; j += step) {
	  EP464_INT j_0 = j >> 3;
	  char j_1 = 1 << (j & 7);
	  is_one_or_square_free[j_0] &= ~j_1;
	}
      }
    } // if prime
  }
  EP464_INT *x = (void*)(prime_sieve) + bit_array_sz;
  EP464_INT n = 0, p = 0;
  for (i = 0; i <= k; ++i) {
    EP464_INT i_0 = i >> 3;
    char i_1 = 1 << (i & 7);
    if (is_one_or_square_free[i_0] & i_1) {
      if (num_factors_mod_2[i_0] & i_1)
	++n;
      else
	++p;
    }
    EP464_INT diff = p - n;
    diff *= 99;
    x[2 * i] = p + diff;
    x[2 * i + 1] = n - diff;
  }
  EP464_INT ret = outer(1, k + 1, x);
  return ret;
}

